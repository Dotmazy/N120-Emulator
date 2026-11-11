#include "HostService.h"
#include "core/Firmware.h"
#include <QCryptographicHash>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QRegularExpression>
#include <QSaveFile>
#include <QTextStream>

static bool writeAtomic(const QString& path, const QByteArray& bytes) {
    QSaveFile out(path);
    return out.open(QIODevice::WriteOnly) && out.write(bytes) == bytes.size() && out.commit();
}

HostService::HostService(QString directory, QString executable, QByteArray secret, QSslCertificate cert,
                         QSslKey key)
    : storage(std::move(directory)), renode(std::move(executable)), token(std::move(secret)),
      certificate(std::move(cert)), privateKey(std::move(key)) {
    QDir().mkpath(storage);
    lease.setSingleShot(true);
    lease.setInterval(10000);
    connect(&lease, &QTimer::timeout, this, [this] {
        if (peer)
            peer->abort();
    });
    checkpoint.setInterval(15000);
    connect(&checkpoint, &QTimer::timeout, this, [this] {
        if (engine.connected())
            engine.command("save " + file("persistent.bin").toUtf8().toBase64());
    });
    checkpoint.start();
    connect(&engine, &RenodeManager::snapshot, this, [this](QJsonObject data) {
        if (!authenticated || !peer || peer->bytesToWrite() > 512 * 1024)
            return;
        const auto frame = data["frame"].toString().toLatin1();
        const auto hash = QCryptographicHash::hash(frame, QCryptographicHash::Sha256);
        if (lastFrame == hash)
            data.remove("frame");
        else
            lastFrame = hash;
        send({{"type", "snapshot"}, {"data", data}});
    });
    connect(&engine, &RenodeManager::state, this, [this](const QString& text) {
        if (!engine.running())
            reloadCheckpoint.clear();
        send({{"type", "state"}, {"text", text}, {"running", engine.running()}});
        if (restartPending && text.startsWith("Stopped")) {
            restartPending = false;
            QTimer::singleShot(0, this, [this] { startEngine(currentLcd); });
        }
    });
    connect(&engine, &RenodeManager::log, this, [this](const QString& text) {
        // Logs stay private to the paired controller; no monitor command endpoint is exported.
        send({{"type", "log"}, {"text", text.left(8192)}});
    });
    connect(&engine, &RenodeManager::response, this, [this](const QJsonObject& response) {
        if (response["reloaded"].toBool()) {
            reloadCheckpoint.clear();
            engine.command("save " + file("persistent.bin").toUtf8().toBase64());
            send({{"type", "reloaded"}});
        }
        if (!reloadCheckpoint.isEmpty() &&
            response["savedPath"].toString().toLatin1() == reloadCheckpoint.toUtf8().toBase64()) {
            engine.command("reload " + file("internal.bin").toUtf8().toBase64() + " - 0 " +
                           reloadCheckpoint.toUtf8().toBase64());
        }
        if (response["saved"].toBool() && exporting) {
            exporting = false;
            QFile flash(file("persistent.bin"));
            if (flash.open(QIODevice::ReadOnly))
                send({{"type", "flash"}, {"data", QString::fromLatin1(flash.readAll().toBase64())}});
            else
                error("Cannot read saved flash");
        }
        if (!response["ok"].toBool()) {
            reloadCheckpoint.clear();
            exporting = false;
            error(response["error"].toString());
        }
    });
}

QString HostService::file(const QString& name) const {
    return QDir(storage).filePath(name);
}

void HostService::send(QJsonObject p) {
    if (authenticated && channel)
        channel->send(p);
}

void HostService::error(const QString& text) {
    send({{"type", "error"}, {"error", text}});
}

void HostService::releaseKeys() {
    for (int i = 0; i < 54; ++i)
        engine.setKey(i, false);
}

void HostService::shutdown() {
    close();
    releaseKeys();
    engine.stop();
}

void HostService::incomingConnection(qintptr descriptor) {
    auto socket = new QSslSocket(this);
    if (!socket->setSocketDescriptor(descriptor)) {
        socket->deleteLater();
        return;
    }
    if (peer) {
        socket->abort();
        socket->deleteLater();
        return;
    }
    peer = socket;
    authenticated = false;
    lastFrame.clear();
    socket->setReadBufferSize(PacketChannel::maxPacket + 4);
    socket->setLocalCertificate(certificate);
    socket->setPrivateKey(privateKey);
    socket->setPeerVerifyMode(QSslSocket::VerifyNone);
    socket->setProtocol(QSsl::TlsV1_2OrLater);
    connect(socket, &QSslSocket::readyRead, this, [this] {
        if (authenticated)
            lease.start();
    });
    connect(socket, &QSslSocket::encrypted, this, [this, socket] {
        channel = new PacketChannel(socket, socket);
        connect(channel, &PacketChannel::packet, this, &HostService::receive);
        connect(channel, &PacketChannel::invalid, socket, &QSslSocket::abort);
    });
    connect(socket, &QSslSocket::disconnected, this, [this, socket] {
        if (authenticated) {
            releaseKeys();
            engine.pause();
            engine.command("save " + file("persistent.bin").toUtf8().toBase64());
        }
        lease.stop();
        reloadCheckpoint.clear();
        authenticated = false;
        exporting = false;
        peer = nullptr;
        channel = nullptr;
        socket->deleteLater();
    });
    lease.start();
    socket->startServerEncryption();
}

void HostService::startEngine(quint32 lcd) {
    if (!n120::validLcd(lcd)) {
        error("Unsupported LCD identity");
        return;
    }
    if (engine.running()) {
        error("Stop the engine before starting it again");
        return;
    }
    Configuration c;
    c.renode = renode;
    c.internal = file("internal.bin");
    c.lcdId = lcd;
    currentLcd = lcd;
    if (!QFileInfo::exists(file("persistent.bin")) &&
        !writeAtomic(file("persistent.bin"), QByteArray(n120::flashCapacity, char(0xFF)))) {
        error("Cannot initialize persistent flash");
        return;
    }
    if (QFileInfo::exists(file("persistent.bin")))
        c.persistent = file("persistent.bin");
    engine.start(c);
}

void HostService::receive(const QJsonObject& p) {
    const auto type = p["type"].toString();
    if (!authenticated) {
        const auto candidate = p["token"].toString().toUtf8();
        // Fixed-size, constant-time comparison. The token is sent only inside pinned TLS.
        unsigned mismatch = unsigned(candidate.size() ^ token.size());
        for (qsizetype i = 0; i < token.size(); ++i)
            mismatch |= unsigned(uchar(token[i]) ^ (i < candidate.size() ? uchar(candidate[i]) : 0));
        if (type != "auth" || mismatch) {
            if (peer)
                peer->disconnectFromHost();
            return;
        }
        authenticated = true;
        send({{"type", "welcome"},
              {"execution", "remote"},
              {"lcdWidth", n120::lcdWidth},
              {"lcdHeight", n120::lcdHeight},
              {"running", engine.running()}});
        send({{"type", "state"},
              {"text", engine.running() ? "Remote session paused; Resume to continue" : "Remote host ready"},
              {"running", engine.running()}});
        lease.start();
        return;
    }
    lease.start();
    if (type == "ping") {
        send({{"type", "pong"}});
        return;
    }
    if (type == "start") {
        startEngine(quint32(p["lcdId"].toDouble(0x4E4101)));
        return;
    }
    if (type == "stop") {
        releaseKeys();
        engine.stop();
        return;
    }
    if (type == "restart") {
        if (!engine.running()) {
            startEngine(currentLcd);
            return;
        }
        releaseKeys();
        restartPending = true;
        engine.stop();
        return;
    }
    if (type == "reload") {
        if (!engine.connected()) {
            error("Start emulation before reloading");
            return;
        }
        releaseKeys();
        if (!reloadCheckpoint.isEmpty()) {
            error("A firmware reload is already pending");
            return;
        }
        // Correlate the checkpoint acknowledgement so a failed write cannot reload stale flash.
        reloadCheckpoint = file("reload-checkpoint.bin");
        engine.command("save " + reloadCheckpoint.toUtf8().toBase64());
        return;
    }
    if (type == "import") {
        if (engine.running()) {
            error("Stop emulation before importing firmware or flash");
            return;
        }
        const auto kind = p["kind"].toString();
        if (kind != "internal" && kind != "slotA" && kind != "slotB" && kind != "flash") {
            error("Unknown image destination");
            return;
        }
        const auto encoded = p["data"].toString().toLatin1();
        auto decoded = QByteArray::fromBase64Encoding(encoded, QByteArray::AbortOnBase64DecodingErrors);
        if (!decoded || decoded.decoded.isEmpty()) {
            error("Invalid or empty image");
            return;
        }
        auto bytes = decoded.decoded;
        const auto validation =
            kind == "internal"
                ? n120::validateInternal(
                      {reinterpret_cast<const std::uint8_t*>(bytes.constData()), std::size_t(bytes.size())})
                : n120::validateExternal(std::size_t(bytes.size()), kind == "slotB" ? 0x400000 : 0);
        if (!validation.empty()) {
            error(QString::fromStdString(validation));
            return;
        }
        if (kind == "internal") {
            if (!writeAtomic(file("internal.bin"), bytes)) {
                error("Cannot store internal firmware");
                return;
            }
        } else {
            QByteArray flash(n120::flashCapacity, char(0xFF));
            QFile existing(file("persistent.bin"));
            if (kind != "flash" && existing.open(QIODevice::ReadOnly)) {
                const auto old = existing.read(n120::flashCapacity);
                std::copy(old.cbegin(), old.cend(), flash.begin());
            }
            const qsizetype offset = kind == "slotB" ? n120::slotCapacity : 0;
            // Import replaces the destination slot, including its erased tail.
            if (kind == "slotB" || (kind == "slotA" && bytes.size() <= qsizetype(n120::slotCapacity)))
                std::fill(flash.begin() + offset, flash.begin() + offset + n120::slotCapacity, char(0xFF));
            std::copy(bytes.cbegin(), bytes.cend(), flash.begin() + offset);
            if (!writeAtomic(file("persistent.bin"), flash)) {
                error("Cannot store external flash");
                return;
            }
        }
        send({{"type", "imported"},
              {"kind", kind},
              {"size", double(bytes.size())},
              {"sha256",
               QString::fromLatin1(QCryptographicHash::hash(bytes, QCryptographicHash::Sha256).toHex())}});
        return;
    }
    if (type == "exportFlash") {
        if (!engine.connected()) {
            QFile flash(file("persistent.bin"));
            if (flash.open(QIODevice::ReadOnly))
                send({{"type", "flash"}, {"data", QString::fromLatin1(flash.readAll().toBase64())}});
            else
                error("No saved external flash exists");
        } else {
            exporting = true;
            engine.command("save " + file("persistent.bin").toUtf8().toBase64());
        }
        return;
    }
    if (type == "control") {
        if (!engine.connected()) {
            error("Start emulation first");
            return;
        }
        const auto command = p["command"].toString();
        static const QRegularExpression allowed(
            "^(pause|resume|reset|key [0-8] [0-5] [01]|battery (100|[0-9]{1,2})|cable [0-2]|speed "
            "(0|0\\.[0-9]{1,3}|[1-9](\\.[0-9]{1,3})?|10)|sequence [0-9,]{1,1024})$");
        if (!allowed.match(command).hasMatch()) {
            error("Unsupported remote control");
            return;
        }
        engine.command(command.toUtf8());
        return;
    }
    error("Unsupported remote request");
}
