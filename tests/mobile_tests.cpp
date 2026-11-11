#include "MobileController.h"
#include "LcdItem.h"
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QSaveFile>
#include <QSignalSpy>
#include <QStandardPaths>
#include <QTemporaryDir>
#include <QtEndian>
#include <QtTest>

class MobileTests : public QObject {
    Q_OBJECT
  private slots:

    void settingsSelectorChangesTheActualSkin() {
        QStandardPaths::setTestModeEnabled(true);
        QTemporaryDir directory;
        QSettings::setDefaultFormat(QSettings::IniFormat);
        QSettings::setPath(QSettings::IniFormat,QSettings::UserScope,directory.path());
        qmlRegisterType<LcdItem>("N120.Display",1,0,"LcdItem");
        qmlRegisterUncreatableType<MobileController>("N120.Display",1,0,"MobileController","Owned by test");
        MobileController controller;
        QQmlApplicationEngine engine;
        engine.rootContext()->setContextProperty("emulator",&controller);
        engine.load(QUrl::fromLocalFile(QString::fromUtf8(N120_MOBILE_QML_SOURCE)));
        QVERIFY(!engine.rootObjects().isEmpty());
        auto choice=engine.rootObjects().first()->findChild<QObject*>("calculatorStyleChoice");
        QVERIFY(choice);
        QCOMPARE(choice->property("count").toInt(),3);
        const QStringList ids={"epsilon","upsilon","omega"};
        for(int i=0;i<ids.size();++i) {
            QVERIFY(choice->setProperty("currentIndex",i));
            QVERIFY(QMetaObject::invokeMethod(choice,"activated",Q_ARG(int,i)));
            QCOMPARE(controller.calculatorStyle(),ids[i]);
            QCOMPARE(choice->property("currentText").toString().toLower(),ids[i]);
            QCOMPARE(controller.setting("appearance/calculatorStyle").toString(),ids[i]);
        }
    }

    void stylesPersistAndReleaseTouches() {
        QStandardPaths::setTestModeEnabled(true);
        QTemporaryDir directory;
        QSettings::setDefaultFormat(QSettings::IniFormat);
        QSettings::setPath(QSettings::IniFormat, QSettings::UserScope, directory.path());
        MobileController controller;
        QCOMPARE(controller.calculatorStyle(), QString("upsilon"));
        controller.touch(1, 126, 1030, true);
        QVERIFY(controller.held().contains(0));
        controller.setCalculatorStyle("omega");
        QVERIFY(controller.held().isEmpty());
        QCOMPARE(controller.calculatorSkin(), QString("qrc:/assets/omega-background.png"));
        MobileController restored;
        QCOMPARE(restored.calculatorStyle(), QString("omega"));
        restored.setCalculatorStyle("epsilon");
        QCOMPARE(restored.calculatorSkin(), QString("qrc:/assets/epsilon-background.png"));
    }

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
