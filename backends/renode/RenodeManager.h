#pragma once
#include "interfaces/EmulatorBackend.h"
#include <QElapsedTimer>
#include <QJsonObject>
#include <QObject>
#include <QProcess>
#include <QTcpSocket>
#include <QTemporaryDir>
#include <QTimer>
#include <memory>

class RenodeManager final : public EmulatorBackend {
    Q_OBJECT
  public:
    explicit RenodeManager(QObject* parent = nullptr);
    ~RenodeManager() override;
    void start(const Configuration&) override;
    void stop() override;
    void command(const QByteArray&) override;
    bool connected() const override;

    bool running() const override {
        return process.state() != QProcess::NotRunning;
    }

    quint16 consolePort() const {
        return monitorPort;
    }

    void setRefreshInterval(int ms) {
        poll.setInterval(qBound(50, ms, 2000));
    }

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
