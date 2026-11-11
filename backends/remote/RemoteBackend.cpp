#include "RemoteBackend.h"
#include <QCryptographicHash>
#include <QSslCertificate>

RemoteBackend::RemoteBackend(QObject* parent) : EmulatorBackend(parent) {
    heartbeat.setInterval(2000);
    deadline.setSingleShot(true);
    deadline.setInterval(10000);
    connect(&socket, &QSslSocket::readyRead, this, [this] { deadline.start(); });
    connect(&heartbeat, &QTimer::timeout, this, [this] { send({{"type", "ping"}}); });
    connect(&deadline, &QTimer::timeout, this, [this] {
        emit log("Remote host timed out");
        disconnectHost();
    });
    connect(&socket, &QSslSocket::encrypted, this, [this] {
        auto actual =
            QString::fromLatin1(socket.peerCertificate().digest(QCryptographicHash::Sha256).toHex());
        if (actual != expectedPin) {
            emit log("Host certificate fingerprint does not match the paired host");
            disconnectHost();
            return;
        }
        channel = std::make_unique<PacketChannel>(&socket);
        connect(channel.get(), &PacketChannel::invalid, this, [this](const QString& error) {
            emit log(error);
            disconnectHost();
        });
        connect(channel.get(), &PacketChannel::packet, this, [this](const QJsonObject& p) {
            deadline.start();
            const auto type = p["type"].toString();
            if (type == "welcome") {
                authenticated = true;
                heartbeat.start();
                emit connectionChanged();
                emit state("Connected · remote execution on Windows");
            } else if (type == "snapshot" && authenticated) {
                engineRunning = true;
                emit snapshot(p["data"].toObject());
            } else if (type == "state" && authenticated) {
                engineRunning = p["running"].toBool();
                emit state(p["text"].toString());
            } else if (type == "error")
                emit log(p["error"].toString());
            emit message(p);
        });
        channel->send({{"type", "auth"}, {"token", secret}});
    });
    connect(&socket, &QSslSocket::disconnected, this, [this] {
        authenticated = engineRunning = false;
        heartbeat.stop();
        deadline.stop();
        emit connectionChanged();
        emit state("Disconnected · remote host required");
    });
    connect(&socket, &QSslSocket::errorOccurred, this, [this] { emit log(socket.errorString()); });
}

void RemoteBackend::connectHost(const QString& host, quint16 port, const QString& pin, const QString& token) {
    disconnectHost();
    expectedPin = pin.toLower().remove(':').remove(' ');
    if (expectedPin.size() != 64 || token.size() < 32 || host.trimmed().isEmpty() || port == 0) {
        emit log(
            "Enter a host, port, 64-digit SHA-256 fingerprint and pairing token (at least 32 characters)");
        return;
    }
    secret = token;
    // Trust is the explicitly paired SHA-256 certificate, checked before any token is sent.
    socket.setPeerVerifyMode(QSslSocket::VerifyNone);
    socket.setProtocol(QSsl::TlsV1_2OrLater);
    socket.connectToHostEncrypted(host.trimmed(), port);
    deadline.start();
    emit state("Connecting securely…");
}

void RemoteBackend::disconnectHost() {
    heartbeat.stop();
    deadline.stop();
    socket.abort();
    authenticated = engineRunning = false;
    // Do not delete a PacketChannel while one of its signals is on the stack.
    if (channel)
        channel.release()->deleteLater();
    emit connectionChanged();
}

void RemoteBackend::send(QJsonObject p) {
    if (authenticated && channel)
        channel->send(p);
}

void RemoteBackend::start(const Configuration& c) {
    send({{"type", "start"}, {"lcdId", double(c.lcdId)}});
}

void RemoteBackend::stop() {
    send({{"type", "stop"}});
}

void RemoteBackend::command(const QByteArray& c) {
    send({{"type", "control"}, {"command", QString::fromUtf8(c)}});
}
