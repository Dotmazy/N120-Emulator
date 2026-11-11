#pragma once
#include <QIODevice>
#include <QJsonObject>
#include <QObject>

// Protocol v1: uint32 big-endian byte count, then UTF-8 JSON. All packets travel over pinned TLS.
class PacketChannel final : public QObject {
    Q_OBJECT
  public:
    static constexpr quint32 maxPacket = 12 * 1024 * 1024;
    explicit PacketChannel(QIODevice* device, QObject* parent = nullptr);
    bool send(QJsonObject packet);
  signals:
    void packet(const QJsonObject&);
    void invalid(const QString&);

  private:
    void read();
    QIODevice* device;
    QByteArray input;
    bool failed = false;
};
