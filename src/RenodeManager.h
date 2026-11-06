#pragma once
#include <QElapsedTimer>
#include <QJsonObject>
#include <QObject>
#include <QProcess>
#include <QTcpSocket>
#include <QTemporaryDir>
#include <QTimer>
#include <memory>

struct Configuration {
    QString renode;
    QString internal;
    QString external;
    QString persistent;
    quint32 lcdId = 0x4E4101;
    quint32 externalOffset = 0;
    bool boot0 = false;
    QString systemRom;
};

class RenodeManager final : public QObject {
    Q_OBJECT
  public:
    explicit RenodeManager(QObject* parent = nullptr);
    ~RenodeManager() override;
    void start(const Configuration&);
    void stop();
    void command(const QByteArray&);
    bool connected() const;
    bool running() const {
        return process.state() != QProcess::NotRunning;
    }
    quint16 consolePort() const {
        return monitorPort;
    }
    void setRefreshInterval(int ms) {
        poll.setInterval(qBound(50, ms, 2000));
    }
  signals:
    void snapshot(const QJsonObject&);
    void log(const QString&);
    void state(const QString&);
    void response(const QJsonObject&);

  private:
    void read();
    void finish();
    QProcess process;
    QTcpSocket socket;
    QTimer poll, retry, deadline;
    QByteArray incoming;
    QElapsedTimer snapshotWait;
    std::unique_ptr<QTemporaryDir> session;
    quint16 port = 0;
    quint16 monitorPort = 0;
    bool pending = false, stopping = false;
    QString persistencePath;
};
