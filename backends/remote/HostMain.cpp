#include "HostService.h"
#include <QCommandLineParser>
#include <QCoreApplication>
#include <QCryptographicHash>
#include <QFile>
#include <QTextStream>
#include <csignal>

static volatile std::sig_atomic_t interrupted = 0;

static void requestStop(int) {
    interrupted = 1;
}

int main(int argc, char** argv) {
    QCoreApplication app(argc, argv);
    app.setApplicationName("n120-host");
    QCommandLineParser parser;
    parser.setApplicationDescription(
        "Dedicated authenticated N120 remote firmware host. No firmware is bundled.");
    parser.addHelpOption();
    for (const auto& name : {"renode", "storage", "cert", "key", "token-file"})
        parser.addOption(QCommandLineOption(name, name, "path"));
    parser.addOption(QCommandLineOption("listen", "Bind address (explicit LAN address for phones)", "address",
                                        "127.0.0.1"));
    parser.addOption(QCommandLineOption("port", "TLS port", "port", "7420"));
    parser.process(app);
    auto read = [&](const QString& option) {
        QFile f(parser.value(option));
        return f.open(QIODevice::ReadOnly) ? f.readAll() : QByteArray();
    };
    const auto token = read("token-file").trimmed();
    const QSslCertificate cert(read("cert"));
    const QSslKey key(read("key"), QSsl::Rsa);
    if (token.size() < 32 || cert.isNull() || key.isNull() || parser.value("storage").isEmpty() ||
        parser.value("renode").isEmpty()) {
        QTextStream(stderr)
            << "Provide --renode --storage --cert --key --token-file (token >=32 characters).\n";
        return 2;
    }
    HostService service(parser.value("storage"), parser.value("renode"), token, cert, key);
    bool validPort = false;
    const auto port = parser.value("port").toUInt(&validPort);
    if (!validPort || port == 0 || port > 65535 ||
        !service.listen(QHostAddress(parser.value("listen")), quint16(port))) {
        QTextStream(stderr) << "Cannot listen: " << service.errorString() << '\n';
        return 3;
    }
    QTextStream(stdout) << "N120 TLS host listening on " << service.serverAddress().toString() << ':'
                        << service.serverPort()
                        << "\nCertificate SHA-256: " << cert.digest(QCryptographicHash::Sha256).toHex()
                        << "\nPairing token is in the supplied token file. Keep it private.\n"
                        << Qt::flush;
    QObject::connect(&app, &QCoreApplication::aboutToQuit, &service, &HostService::shutdown);
    std::signal(SIGINT, requestStop);
    std::signal(SIGTERM, requestStop);
    QTimer shutdownPoll;
    shutdownPoll.setInterval(100);
    QObject::connect(&shutdownPoll, &QTimer::timeout, &app, [&] {
        if (interrupted) {
            interrupted = 0;
            shutdownPoll.stop();
            service.shutdown();
            QTimer::singleShot(2500, &app, &QCoreApplication::quit);
        }
    });
    shutdownPoll.start();
    return app.exec();
}
