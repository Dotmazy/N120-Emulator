#include "RenodeManager.h"
#include <QCoreApplication>
#include <QFile>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QtEndian>
#include <QtTest>

class ManagerTests : public QObject {
    Q_OBJECT
  private slots:

    void invalidVectors_data() {
        QTest::addColumn<quint32>("sp");
        QTest::addColumn<quint32>("pc");
        QTest::newRow("unmapped-stack-gap") << quint32(0x21000000) << quint32(0x08000009);
        QTest::newRow("unaligned-stack") << quint32(0x24001001) << quint32(0x08000009);
        QTest::newRow("pc-outside-image") << quint32(0x24001000) << quint32(0x08000101);
        QTest::newRow("non-thumb-pc") << quint32(0x24001000) << quint32(0x08000008);
    }

    void invalidVectors() {
        QFETCH(quint32, sp);
        QFETCH(quint32, pc);
        QTemporaryDir directory;
        QVERIFY(directory.isValid());
        QFile image(directory.filePath("internal.bin"));
        QVERIFY(image.open(QIODevice::WriteOnly));
        QByteArray bytes(32, char(0));
        qToLittleEndian(sp, bytes.data());
        qToLittleEndian(pc, bytes.data() + 4);
        QCOMPARE(image.write(bytes), bytes.size());
        image.close();
        Configuration configuration;
        configuration.renode = QCoreApplication::applicationFilePath();
        configuration.internal = image.fileName();
        RenodeManager manager;
        QSignalSpy state(&manager, &RenodeManager::state);
        QSignalSpy log(&manager, &RenodeManager::log);
        manager.start(configuration);
        QVERIFY(!manager.running());
        QCOMPARE(state.last().at(0).toString(), QString("Configuration error"));
        QVERIFY(log.last().at(0).toString().contains("reset vector"));
    }

    void missingEngine() {
        Configuration configuration;
        RenodeManager manager;
        QSignalSpy state(&manager, &RenodeManager::state);
        manager.start(configuration);
        QVERIFY(!manager.running());
        QCOMPARE(state.last().at(0).toString(), QString("Configuration error"));
    }
};
QTEST_GUILESS_MAIN(ManagerTests)
#include "manager_tests.moc"
