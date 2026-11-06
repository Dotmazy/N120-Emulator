#include "RenodeManager.h"
#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonDocument>
#include <QTcpServer>
#include <QtEndian>
#include <stdexcept>

static QString pathToken(QString p) {
    p = QDir::fromNativeSeparators(p);
    if (p.contains('\n') || p.contains('\r') || p.contains('"'))
        throw std::runtime_error("Unsupported character in filename");
    p.replace(" ", "\\ ");
    return "@" + p;
}
static QString stringToken(QString p) {
    p = QDir::fromNativeSeparators(p);
    if (p.contains('"') || p.contains('\n') || p.contains('\r'))
        throw std::runtime_error("Unsupported character in filename");
    return "\"" + p + "\"";
}
RenodeManager::RenodeManager(QObject* parent) : QObject(parent) {
    poll.setInterval(100);
    retry.setInterval(250);
    deadline.setSingleShot(true);
    deadline.setInterval(30000);
    connect(&poll, &QTimer::timeout, this, [this] {
        if (connected() && !pending) {
            pending = true;
            snapshotWait.start();
            command("snapshot");
        } else if (connected() && pending && snapshotWait.isValid() && snapshotWait.elapsed() > 5000) {
            emit log(
                "[WARNING] Hardware snapshot stalled; reconnecting the bridge without resetting firmware.");
            socket.abort();
            retry.start();
        }
    });
    connect(&retry, &QTimer::timeout, this, [this] {
        if (socket.state() == QAbstractSocket::UnconnectedState)
            socket.connectToHost(QHostAddress::LocalHost, port);
    });
    connect(&deadline, &QTimer::timeout, this, [this] {
        emit log("Renode bridge timed out; inspect the engine log.");
        stop();
    });
    connect(&socket, &QTcpSocket::connected, this, [this] {
        incoming.clear();
        retry.stop();
        deadline.stop();
        poll.start();
        emit state("Connected");
        command("snapshot");
        pending = true;
        snapshotWait.start();
    });
    connect(&socket, &QTcpSocket::readyRead, this, &RenodeManager::read);
    connect(&socket, &QTcpSocket::disconnected, this, [this] {
        poll.stop();
        pending = false;
        if (!stopping) {
            emit state("Disconnected");
            if (running())
                retry.start();
        }
    });
    connect(&process, &QProcess::readyReadStandardOutput, this,
            [this] { emit log(QString::fromUtf8(process.readAllStandardOutput())); });
    connect(&process, &QProcess::readyReadStandardError, this,
            [this] { emit log(QString::fromUtf8(process.readAllStandardError())); });
    connect(&process, &QProcess::errorOccurred, this, [this](QProcess::ProcessError error) {
        if (error == QProcess::FailedToStart)
            finish();
        emit log(process.errorString());
        emit state("Engine error");
    });
    connect(&process, qOverload<int, QProcess::ExitStatus>(&QProcess::finished), this,
            [this](int code, QProcess::ExitStatus) {
                finish();
                emit state(QString("Stopped (exit %1)").arg(code));
            });
}
RenodeManager::~RenodeManager() {
    stopping = true;
    poll.stop();
    retry.stop();
    deadline.stop();
    if (connected()) {
        if (!persistencePath.isEmpty())
            command("save " + persistencePath.toUtf8().toBase64());
        command("shutdown");
        socket.flush();
        socket.waitForBytesWritten(200);
    }
    if (process.state() != QProcess::NotRunning && !process.waitForFinished(1500)) {
        process.kill();
        process.waitForFinished(1000);
    }
}
bool RenodeManager::connected() const {
    return socket.state() == QAbstractSocket::ConnectedState;
}
void RenodeManager::command(const QByteArray& cmd) {
    if (connected())
        socket.write(cmd + '\n');
}
void RenodeManager::start(const Configuration& c) {
    if (process.state() != QProcess::NotRunning) {
        emit log("Stop the engine before restarting.");
        return;
    }
    try {
        if (!QFileInfo::exists(c.renode))
            throw std::runtime_error("Renode executable not found");
        QFile internal(c.internal);
        if (!internal.open(QIODevice::ReadOnly))
            throw std::runtime_error("Cannot read internal firmware");
        if (internal.size() < 8 || internal.size() > 0x80000)
            throw std::runtime_error("Internal firmware must contain vectors and fit 512 KiB");
        auto bytes = internal.readAll();
        if (bytes.size() < 8 || bytes.size() > 0x80000)
            throw std::runtime_error("Internal firmware must contain vectors and fit 512 KiB");
        const quint32 sp = qFromLittleEndian<quint32>(bytes.constData());
        const quint32 pc = qFromLittleEndian<quint32>(bytes.constData() + 4);
        if (!(pc & 1) || pc < 0x08000001 || pc >= 0x08000000 + quint32(bytes.size()) || (sp & 7) ||
            !((sp > 0x20000000 && sp <= 0x20020000) || (sp > 0x24000000 && sp <= 0x24050000) ||
              (sp > 0x30000000 && sp <= 0x30008000) || (sp > 0x38000000 && sp <= 0x38004000)))
            throw std::runtime_error("Invalid H725 reset vector");
        if (c.externalOffset != 0 && c.externalOffset != 0x400000)
            throw std::runtime_error("External image destination must be Slot A or Slot B");
        if (!c.persistent.isEmpty()) {
            const auto destination = QFileInfo(c.persistent).canonicalFilePath();
            if (!destination.isEmpty() && (destination == QFileInfo(c.internal).canonicalFilePath() ||
                                           destination == QFileInfo(c.external).canonicalFilePath()))
                throw std::runtime_error("Saved flash must use a separate file from input firmware images");
        }
        for (const auto& input : {qMakePair(c.external, qint64(0x800000 - c.externalOffset)),
                                  qMakePair(c.persistent, qint64(0x800000))}) {
            if (input.first.isEmpty())
                continue;
            QFile file(input.first);
            if (!file.open(QIODevice::ReadOnly))
                throw std::runtime_error("Cannot read external or persistent flash image");
            if (file.size() > input.second)
                throw std::runtime_error("External or persistent flash image exceeds its mapping");
        }
        quint32 revision =
            bytes.size() >= 0x7FA04 ? qFromLittleEndian<quint32>(bytes.constData() + 0x7FA00) : 0xFFFFFFFF;
        session = std::make_unique<QTemporaryDir>();
        if (!session->isValid())
            throw std::runtime_error("Cannot create session directory");
        QString resources = QCoreApplication::applicationDirPath() + "/renode";
        QFile platform(resources + "/n120.repl");
        if (!platform.open(QIODevice::ReadOnly))
            throw std::runtime_error("Missing Renode platform resources");
        auto text = platform.readAll();
        text.replace("\r\n", "\n");
        text.replace("lcd: N120.Lcd @ sysbus 0x60000000\n    board: board",
                     QByteArray("lcd: N120.Lcd @ sysbus 0x60000000\n    board: board\n    id: ") +
                         QByteArray::number(c.lcdId));
        const QString repl = session->filePath("n120.repl");
        QFile out(repl);
        if (!out.open(QIODevice::WriteOnly) || out.write(text) != text.size())
            throw std::runtime_error("Cannot write session platform");
        out.close();
        if (c.boot0) {
            QFile rom(c.systemRom);
            if (!rom.open(QIODevice::ReadOnly))
                throw std::runtime_error("BOOT0 needs a user-supplied STM32H725 system ROM image");
            if(rom.size()<8||rom.size()>0x20000)
                throw std::runtime_error("System ROM must contain vectors and fit the 128 KiB mapping");
            const auto image = rom.readAll();
            if (image.size() < 8 || image.size() > 0x20000)
                throw std::runtime_error("System ROM must contain vectors and fit the 128 KiB mapping");
            const auto romSp = qFromLittleEndian<quint32>(image.constData()),
                       romPc = qFromLittleEndian<quint32>(image.constData() + 4);
            const bool stackMapped =
                (romSp > 0x20000000 && romSp <= 0x20020000) || (romSp > 0x24000000 && romSp <= 0x24050000) ||
                (romSp > 0x30000000 && romSp <= 0x30008000) || (romSp > 0x38000000 && romSp <= 0x38004000);
            if ((romSp & 7) || !stackMapped || !(romPc & 1) || romPc < 0x1FF00001 ||
                romPc >= 0x1FF00000 + quint32(image.size()))
                throw std::runtime_error("Invalid STM32H725 system ROM vectors");
        }
        QTcpServer allocator;
        if (!allocator.listen(QHostAddress::LocalHost, 0))
            throw std::runtime_error("Cannot allocate bridge port");
        port = allocator.serverPort();
        QTcpServer monitorAllocator;
        if (!monitorAllocator.listen(QHostAddress::LocalHost, 0))
            throw std::runtime_error("Cannot allocate console port");
        monitorPort = monitorAllocator.serverPort();
        monitorAllocator.close();
        allocator.close();
        QString script = "include " + pathToken(resources + "/UsbHost.cs") + "\ninclude " +
                         pathToken(resources + "/N120.cs") +
                         "\nmach create \"N120\"\nmachine LoadPlatformDescription " + pathToken(repl) +
                         "\nsysbus LoadBinary " + pathToken(c.internal) + " 0x08000000\n";
        if (c.boot0)
            script += "sysbus LoadBinary " + pathToken(c.systemRom) + " 0x1FF00000\nlcd ConfigureBoot true\n";
        if (!c.external.isEmpty())
            script += "ospi Load " + stringToken(c.external) + " " + QString::number(c.externalOffset) + "\n";
        if (!c.persistent.isEmpty())
            script += "ospi Load " + stringToken(c.persistent) + "\n";
        script += "lcd ConfigureRevision " + QString::number(revision) +
                  "\nospi ProtectMapping\nlcd StartBridge " + QString::number(port) + "\nstart\n";
        QFile resc(session->filePath("start.resc"));
        if (!resc.open(QIODevice::WriteOnly) || resc.write(script.toUtf8()) < 0)
            throw std::runtime_error("Cannot write startup script");
        resc.close();
        stopping = false;
        pending = false;
        incoming.clear();
        socket.abort();
        persistencePath = c.persistent;
        process.setProgram(c.renode);
        process.setArguments({"--config", session->filePath("renode.config"), "--disable-gui",
                              "--hide-monitor", "--plain", "--port", QString::number(monitorPort),
                              resc.fileName()});
        process.setWorkingDirectory(session->path());
        process.start();
        retry.start();
        deadline.start();
        emit state("Starting Renode…");
    } catch (const std::exception& e) {
        emit log(QString::fromUtf8(e.what()));
        emit state("Configuration error");
    }
}
void RenodeManager::stop() {
    stopping = true;
    poll.stop();
    retry.stop();
    deadline.stop();
    if (connected()) {
        if (!persistencePath.isEmpty())
            command("save " + persistencePath.toUtf8().toBase64());
        command("shutdown");
    }
    QTimer::singleShot(2000, this, [this] {
        if (stopping && process.state() != QProcess::NotRunning)
            process.kill();
    });
}
void RenodeManager::finish() {
    poll.stop();
    retry.stop();
    deadline.stop();
    socket.abort();
    pending = false;
}
void RenodeManager::read() {
    incoming += socket.readAll();
    if (incoming.size() > 2 * 1024 * 1024) {
        emit log("Oversized bridge response");
        stop();
        return;
    }
    while (incoming.contains('\n')) {
        auto end = incoming.indexOf('\n');
        auto line = incoming.left(end);
        incoming.remove(0, end + 1);
        QJsonParseError error;
        auto doc = QJsonDocument::fromJson(line, &error);
        if (error.error != QJsonParseError::NoError || !doc.isObject()) {
            emit log("Invalid bridge JSON: " + error.errorString());
            stop();
            return;
        }
        auto obj = doc.object();
        if (obj.contains("frame")) {
            pending = false;
            emit snapshot(obj);
        } else {
            emit response(obj);
            if (!obj["ok"].toBool())
                emit log("Peripheral command rejected: " + obj["error"].toString());
        }
    }
}
