#pragma once
#include "PacketChannel.h"
#include "interfaces/EmulatorBackend.h"
#include <QSslSocket>
#include <QTimer>
#include <memory>

class RemoteBackend final : public EmulatorBackend {
    Q_OBJECT
  public:
    explicit RemoteBackend(QObject* parent = nullptr);
    void connectHost(const QString& host, quint16 port, const QString& pin, const QString& secret);
    void disconnectHost();
    void send(QJsonObject packet);
    void start(const Configuration&) override;
    void stop() override;
    void command(const QByteArray&) override;

    bool connected() const override {
        return authenticated;
    }

    bool running() const override {
        return engineRunning;
    }

    bool connecting() const {
        return socket.state() != QAbstractSocket::UnconnectedState;
    }

  signals:
    void message(const QJsonObject&);
    void connectionChanged();

  private:
    QSslSocket socket;
    std::unique_ptr<PacketChannel> channel;
    QTimer heartbeat, deadline;
    QString expectedPin, secret;
    bool authenticated = false, engineRunning = false;
};
