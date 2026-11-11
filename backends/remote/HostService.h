#pragma once
#include "PacketChannel.h"
#include "RenodeManager.h"
#include <QPointer>
#include <QSslCertificate>
#include <QSslKey>
#include <QSslSocket>
#include <QTcpServer>
#include <QTimer>

class HostService final : public QTcpServer {
    Q_OBJECT
  public:
    HostService(QString storage, QString renode, QByteArray token, QSslCertificate cert, QSslKey key);
    void shutdown();

  protected:
    void incomingConnection(qintptr descriptor) override;

  private:
    void receive(const QJsonObject& packet);
    void send(QJsonObject packet);
    void error(const QString& text);
    void releaseKeys();
    void startEngine(quint32 lcd);
    QString file(const QString& name) const;
    QString storage, renode;
    QByteArray token, lastFrame;
    QSslCertificate certificate;
    QSslKey privateKey;
    QPointer<QSslSocket> peer;
    QPointer<PacketChannel> channel;
    QTimer lease, checkpoint;
    RenodeManager engine;
    bool authenticated = false, exporting = false;
    bool restartPending = false;
    quint32 currentLcd = 0x4E4101;
    QString reloadCheckpoint;
};
