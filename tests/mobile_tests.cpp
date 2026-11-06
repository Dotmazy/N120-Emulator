#include "MobileController.h"
#include <QSaveFile>
#include <QSignalSpy>
#include <QStandardPaths>
#include <QTemporaryDir>
#include <QtEndian>
#include <QtTest>

class MobileTests : public QObject {
    Q_OBJECT
  private slots:

    void touchesReleaseAndReferenceCount() {
        QStandardPaths::setTestModeEnabled(true);
        MobileController controller;
        controller.touch(1, 580, 1090, true);
        QVERIFY(controller.held().contains(8));
        controller.touch(2, 580, 1090, true);
        controller.touch(1, 580, 1090, false);
        QVERIFY(controller.held().contains(8));
        controller.touch(2, 0, 0, true);
        QVERIFY(controller.held().isEmpty());
        controller.touch(1, 190, 1020, true);
        controller.touch(2, 270, 940, true);
        QCOMPARE(controller.held().size(), 2);
        controller.releaseTouches();
        QVERIFY(controller.held().isEmpty());
    }

    void privateImportAndSettings() {
        QStandardPaths::setTestModeEnabled(true);
        QTemporaryDir directory;
        QSettings::setDefaultFormat(QSettings::IniFormat);
        QSettings::setPath(QSettings::IniFormat, QSettings::UserScope, directory.path());
        QFile input(directory.filePath("firmware with spaces.bin"));
        QVERIFY(input.open(QIODevice::WriteOnly));
        QByteArray image(32, char(0));
        qToLittleEndian(quint32(0x24001000), image.data());
        qToLittleEndian(quint32(0x08000009), image.data() + 4);
        input.write(image);
        input.close();
        {
            MobileController controller;
            controller.importFile(QUrl::fromLocalFile(input.fileName()), "internal");
            QVERIFY(controller.error().isEmpty());
            QVERIFY(controller.files().contains("32 bytes"));
            controller.setSetting("battery", 37);
            controller.setSetting("theme", 2);
        }
        MobileController restored;
        QCOMPARE(restored.setting("battery").toInt(), 37);
        QCOMPARE(restored.setting("theme").toInt(), 2);
        QVERIFY(input.open(QIODevice::WriteOnly | QIODevice::Truncate));
        input.write("invalid");
        input.close();
        restored.importFile(QUrl::fromLocalFile(input.fileName()), "internal");
        QVERIFY(!restored.error().isEmpty());
        QVERIFY(restored.files().contains("32 bytes"));
    }
};
QTEST_MAIN(MobileTests)
#include "mobile_tests.moc"
