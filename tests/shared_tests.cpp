#include "PacketChannel.h"
#include "core/Firmware.h"
#include <QSignalSpy>
#include <QTcpServer>
#include <QTcpSocket>
#include <QtEndian>
#include <QtTest>

class SharedTests : public QObject {
    Q_OBJECT
  private slots:

    void vectors() {
        QByteArray bytes(32, char(0));
        qToLittleEndian(quint32(0x24050000), bytes.data());
        qToLittleEndian(quint32(0x08000009), bytes.data() + 4);
        auto validate = [&] {
            return n120::validateInternal(
                {reinterpret_cast<const std::uint8_t*>(bytes.data()), std::size_t(bytes.size())});
        };
        QVERIFY(validate().empty());
        for (quint32 sp : {0x20000000u, 0x20020008u, 0x24050008u, 0x30008008u, 0x38004008u, 0x24001001u}) {
            qToLittleEndian(sp, bytes.data());
            QVERIFY(!validate().empty());
        }
        for (quint32 sp : {0x20020000u, 0x24050000u, 0x30008000u, 0x38004000u}) {
            qToLittleEndian(sp, bytes.data());
            QVERIFY(validate().empty());
        }
        qToLittleEndian(quint32(0x08000021), bytes.data() + 4);
        QVERIFY(!validate().empty());
        bytes.resize(7);
        QVERIFY(!validate().empty());
    }

    void flashBoundsAndLcd() {
        QVERIFY(n120::validateExternal(0x800000, 0).empty());
        QVERIFY(n120::validateExternal(0x400000, 0x400000).empty());
        QVERIFY(!n120::validateExternal(0x400001, 0x400000).empty());
        QVERIFY(!n120::validateExternal(1, 1).empty());
        for (quint32 id : {0x4E4101, 0x4E4102, 0x4E4105, 0x4E4106, 0x4E4601, 0x4E4801, 0x4E5401})
            QVERIFY(n120::validLcd(id));
        QVERIFY(!n120::validLcd(0));
    }

    void fragmentedProtocol() {
        QTcpServer server;
        QVERIFY(server.listen(QHostAddress::LocalHost));
        QTcpSocket client;
        client.connectToHost(QHostAddress::LocalHost, server.serverPort());
        QVERIFY(client.waitForConnected());
        QVERIFY(server.waitForNewConnection());
        auto peer = server.nextPendingConnection();
        PacketChannel channel(peer);
        QSignalSpy packets(&channel, &PacketChannel::packet);
        QByteArray json = "{\"v\":1,\"type\":\"ping\"}";
        QByteArray wire(4, char(0));
        qToBigEndian(quint32(json.size()), wire.data());
        wire += json;
        client.write(wire.left(2));
        client.flush();
        QTest::qWait(10);
        QCOMPARE(packets.size(), 0);
        client.write(wire.mid(2, 5));
        client.flush();
        QTest::qWait(10);
        QCOMPARE(packets.size(), 0);
        client.write(wire.mid(7) + wire);
        client.flush();
        QTRY_COMPARE(packets.size(), 2);
        QSignalSpy invalid(&channel, &PacketChannel::invalid);
        QByteArray oversized(4, char(0));
        qToBigEndian(PacketChannel::maxPacket + 1, oversized.data());
        client.write(oversized);
        client.flush();
        QTRY_COMPARE(invalid.size(), 1);
        client.write(wire);
        client.flush();
        QTest::qWait(10);
        QCOMPARE(packets.size(), 2);
    }
};
QTEST_GUILESS_MAIN(SharedTests)
#include "shared_tests.moc"
