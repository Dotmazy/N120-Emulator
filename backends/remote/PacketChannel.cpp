#include "PacketChannel.h"
#include <QJsonDocument>
#include <QtEndian>

PacketChannel::PacketChannel(QIODevice* d, QObject* p) : QObject(p), device(d) {
    connect(d, &QIODevice::readyRead, this, &PacketChannel::read);
}

bool PacketChannel::send(QJsonObject p) {
    if (failed || !device->isWritable())
        return false;
    p["v"] = 1;
    const auto json = QJsonDocument(p).toJson(QJsonDocument::Compact);
    // Bound queued data as well as individual packets. A blocked peer must not consume unlimited RAM.
    if (json.size() > maxPacket || device->bytesToWrite() > 2 * maxPacket)
        return false;
    QByteArray wire(4, char(0));
    qToBigEndian(quint32(json.size()), wire.data());
    wire += json;
    return device->write(wire) == wire.size();
}

void PacketChannel::read() {
    if (failed)
        return;
    while (device->bytesAvailable()) {
        input += device->read(qMin<qint64>(65536, device->bytesAvailable()));
        while (input.size() >= 4) {
            const auto size = qFromBigEndian<quint32>(input.constData());
            if (size < 2 || size > maxPacket) {
                failed = true;
                emit invalid("Invalid packet size");
                return;
            }
            if (input.size() < size + 4)
                break;
            QJsonParseError error;
            auto doc = QJsonDocument::fromJson(input.mid(4, size), &error);
            input.remove(0, size + 4);
            if (error.error != QJsonParseError::NoError || !doc.isObject() ||
                doc.object()["v"].toInt() != 1) {
                failed = true;
                emit invalid("Invalid packet or unsupported protocol version");
                return;
            }
            emit packet(doc.object());
            if (failed)
                return;
        }
    }
}
