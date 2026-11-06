#include "MainWindow.h"
#include <QAction>
#include <QApplication>
#include <QCheckBox>
#include <QComboBox>
#include <QDialog>
#include <QDir>
#include <QDockWidget>
#include <QFile>
#include <QInputDialog>
#include <QJsonDocument>
#include <QKeyEvent>
#include <QLineEdit>
#include <QPushButton>
#include <QSettings>
#include <QSpinBox>
#include <QStyleOptionSpinBox>
#include <QTabWidget>
#include <QTableWidget>
#include <QTimer>

int main(int argc, char** argv) {
    QApplication app(argc, argv);
    app.setStyle("Fusion");
    QCoreApplication::setOrganizationName("N120Emulator");
    bool usbSmoke = app.arguments().contains("--usb-smoke");
    bool uiSmoke = app.arguments().contains("--ui-smoke"),
         firmwareSmoke = app.arguments().contains("--firmware-smoke") || usbSmoke;
    // Keep smoke-test preferences separate from the user's normal calculator session.
    QCoreApplication::setApplicationName(usbSmoke                   ? "N120StudioUsbTests"
                                         : uiSmoke || firmwareSmoke ? "N120StudioTests"
                                                                    : "N120Emulator");
    MainWindow window(uiSmoke || firmwareSmoke);
    window.show();
    if (uiSmoke)
        QTimer::singleShot(1000, &app, &QApplication::quit);
    const QString outputDir =
        qEnvironmentVariable("N120_TEST_OUTPUT", QCoreApplication::applicationDirPath() + "/test-output");
    if (firmwareSmoke)
        QDir().mkpath(outputDir);
    QJsonObject uiResults;
    if (firmwareSmoke && !usbSmoke) {
        const QList<int> smokeKeys = {4, 4, 4, 6, 2, 2, 2, 3, 4, 4};
        for (int i = 0; i < smokeKeys.size(); ++i) {
            int key = smokeKeys[i];
            QTimer::singleShot(14000 + i * 400, &app, [&, key] {
                window.calculator()->physicalKey(key, true);
                QTimer::singleShot(60, &app, [&, key] { window.calculator()->physicalKey(key, false); });
            });
        }
        QTimer::singleShot(22000, &app, [&] {
            window.findChild<QAction*>("standard")->trigger();
            window.grab().save(outputDir + "/standard-smoke.png");
        });
        QTimer::singleShot(7000, &app, [&] {
            window.findChild<QAction*>("standard")->trigger();
            window.grab().save(outputDir + "/standard-smoke.png");
        });
        QTimer::singleShot(8000, &app, [&] {
            window.findChild<QAction*>("compact")->trigger();
            window.grab().save(outputDir + "/compact-smoke.png");
        });
        QTimer::singleShot(9000, &app, [&] {
            window.findChild<QAction*>("developer")->trigger();
            window.grab().save(outputDir + "/developer-smoke.png");
        });
        QTimer::singleShot(10000, &app, [&] {
            QTimer::singleShot(350, &app, [&] {
                if (auto dialog = qobject_cast<QDialog*>(QApplication::activeModalWidget())) {
                    auto theme = dialog->findChild<QComboBox*>("themeChoice");
                    auto layout = dialog->findChild<QComboBox*>("layoutChoice");
                    auto shortcuts = dialog->findChild<QCheckBox*>("shortcutsEnabled");
                    auto table = dialog->findChild<QTableWidget*>("shortcutTable");
                    auto defaults = dialog->findChild<QPushButton*>("shortcutDefaults");
                    theme->setCurrentText("Light");
                    uiResults["lightTextContrast"] = qApp->palette().color(QPalette::Text).lightness() < 80;
                    dialog->grab().save(outputDir + "/settings-light.png");
                    theme->setCurrentText("Dark");
                    uiResults["darkTextContrast"] = qApp->palette().color(QPalette::Text).lightness() > 180;
                    shortcuts->setChecked(false);
                    uiResults["shortcutsDisabled"] = !table->isEnabled() && !defaults->isEnabled();
                    shortcuts->setChecked(true);
                    uiResults["shortcutsReenabled"] = table->isEnabled() && defaults->isEnabled();
                    layout->setCurrentText("Compact");
                    uiResults["instantCompact"] =
                        window.findChild<QDockWidget*>("hardwareInspector")->isHidden();
                    layout->setCurrentText("Standard");
                    uiResults["instantStandard"] =
                        !window.findChild<QDockWidget*>("hardwareInspector")->isHidden();
                    dialog->grab().save(outputDir + "/settings-dark.png");
                    auto pages = dialog->findChild<QTabWidget*>("settingsPages");
                    pages->setCurrentIndex(5);
                    QCoreApplication::processEvents();
                    auto spin = dialog->findChild<QSpinBox*>("batteryPercent");
                    QStyleOptionSpinBox opt;
                    opt.initFrom(spin);
                    opt.rect = spin->rect();
                    opt.frame = true;
                    opt.buttonSymbols = QAbstractSpinBox::UpDownArrows;
                    opt.subControls = QStyle::SC_All;
                    auto arrow =
                        spin->style()->subControlRect(QStyle::CC_SpinBox, &opt, QStyle::SC_SpinBoxUp, spin);
                    auto edit = spin->findChild<QLineEdit*>();
                    uiResults["batteryEditorClearOfArrows"] = edit && !edit->geometry().intersects(arrow);
                    dialog->grab().save(outputDir + "/settings-battery.png");
                    dialog->reject();
                }
            });
            window.findChild<QAction*>("settings")->trigger();
        });
        QTimer::singleShot(12000, &app, [&] { window.findChild<QAction*>("pause")->trigger(); });
        QTimer::singleShot(12500, &app, [&] { window.findChild<QAction*>("resume")->trigger(); });
    }
    if (usbSmoke) {
        for (int i = 0; i < 3; ++i)
            QTimer::singleShot(7500 + i * 500, &app, [&] {
                window.calculator()->physicalKey(4, true);
                QTimer::singleShot(60, &app, [&] { window.calculator()->physicalKey(4, false); });
            });
        QTimer::singleShot(11000, &app, [&] {
            window.findChild<QAction*>("usb")->trigger();
            QTimer::singleShot(100, &app, [&] {
                if (auto dialog = qobject_cast<QInputDialog*>(QApplication::activeModalWidget())) {
                    dialog->setIntValue(3240);
                    dialog->accept();
                }
            });
            window.findChild<QPushButton*>("usbIpStart")->click();
        });
        QTimer::singleShot(14000, &app, [&] { window.findChild<QPushButton*>("windowsAttach")->click(); });
        QTimer::singleShot(21500, &app, [&] {
            auto state = window.lastSnapshot();
            state.remove("frame");
            state["windowsStatus"] = window.hostUsbStatus();
            uiResults["windowsImported"] = state["usbImported"].toBool();
            uiResults["winUsbBound"] =
                window.hostUsbStatus().contains("WINUSB") && window.hostUsbStatus().contains("OK");
            QFile report(outputDir + "/usb-gui-state.json");
            if (report.open(QIODevice::WriteOnly))
                report.write(QJsonDocument(state).toJson());
            window.grab().save(outputDir + "/usb-gui.png");
        });
        QTimer::singleShot(22500, &app,
                           [&] { window.findChild<QComboBox*>("usbCable")->setCurrentIndex(1); });
        QTimer::singleShot(27000, &app, [&] {
            auto state = window.lastSnapshot();
            uiResults["chargerStopsHost"] = state["usbIpPort"].toInt() == 0 && !state["usbImported"].toBool();
        });
    }
    if (firmwareSmoke) {
        Configuration c;
        c.renode = qEnvironmentVariable("RENODE_PATH");
        c.internal = qEnvironmentVariable("N120_INTERNAL");
        c.external = qEnvironmentVariable("N120_EXTERNAL");
        window.startWith(c);
        QTimer::singleShot(30000, &app, [&] {
            window.grab().save(outputDir + (usbSmoke ? "/usb-gui-final.png" : "/gui-smoke.png"));
            auto state = window.lastSnapshot();
            state.remove("frame");
            state["uiChecks"] = uiResults;
            QFile report(outputDir + (usbSmoke ? "/usb-gui-final.json" : "/gui-smoke-state.json"));
            if (report.open(QIODevice::WriteOnly))
                report.write(QJsonDocument(state).toJson());
            bool uiOk = true;
            for (auto value : uiResults)
                uiOk = uiOk && value.toBool();
            app.exit(window.hasFrame() && uiOk ? 0 : 2);
        });
    }
    return app.exec();
}
