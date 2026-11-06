#include "MainWindow.h"
#include <QApplication>
#include <QTimer>
#include <QFile>
#include <QKeyEvent>
#include <QJsonDocument>
#include <QSettings>
#include <QAction>
#include <QDialog>
#include <QComboBox>
#include <QCheckBox>
#include <QTableWidget>
#include <QPushButton>
#include <QDockWidget>
#include <QSpinBox>
#include <QLineEdit>
#include <QStyleOptionSpinBox>
#include <QTabWidget>
int main(int argc,char** argv){
    QApplication app(argc,argv);app.setStyle("Fusion");QCoreApplication::setOrganizationName("N120Emulator");
    bool uiSmoke=app.arguments().contains("--ui-smoke"),firmwareSmoke=app.arguments().contains("--firmware-smoke");
    QCoreApplication::setApplicationName(uiSmoke||firmwareSmoke?"N120StudioTests":"N120Emulator");
    MainWindow window(uiSmoke||firmwareSmoke);window.show();
    if(uiSmoke)QTimer::singleShot(1000,&app,&QApplication::quit);
    QJsonObject uiResults;
    if(firmwareSmoke){
        const QList<int> smokeKeys={4,4,4,6,2,2,2,3,4,4};for(int i=0;i<smokeKeys.size();++i){int key=smokeKeys[i];QTimer::singleShot(14000+i*400,&app,[&,key]{window.calculator()->physicalKey(key,true);QTimer::singleShot(60,&app,[&,key]{window.calculator()->physicalKey(key,false);});});}
        QTimer::singleShot(22000,&app,[&]{window.findChild<QAction*>("standard")->trigger();window.grab().save(QStringLiteral(N120_SOURCE_DIR)+"/docs/standard-smoke.png");});
        QTimer::singleShot(7000,&app,[&]{window.findChild<QAction*>("standard")->trigger();window.grab().save(QStringLiteral(N120_SOURCE_DIR)+"/docs/standard-smoke.png");});
        QTimer::singleShot(8000,&app,[&]{window.findChild<QAction*>("compact")->trigger();window.grab().save(QStringLiteral(N120_SOURCE_DIR)+"/docs/compact-smoke.png");});
        QTimer::singleShot(9000,&app,[&]{window.findChild<QAction*>("developer")->trigger();});
        QTimer::singleShot(10000,&app,[&]{QTimer::singleShot(350,&app,[&]{if(auto dialog=qobject_cast<QDialog*>(QApplication::activeModalWidget())){auto theme=dialog->findChild<QComboBox*>("themeChoice");auto layout=dialog->findChild<QComboBox*>("layoutChoice");auto shortcuts=dialog->findChild<QCheckBox*>("shortcutsEnabled");auto table=dialog->findChild<QTableWidget*>("shortcutTable");auto defaults=dialog->findChild<QPushButton*>("shortcutDefaults");theme->setCurrentText("Light");uiResults["lightTextContrast"]=qApp->palette().color(QPalette::Text).lightness()<80;dialog->grab().save(QStringLiteral(N120_SOURCE_DIR)+"/docs/settings-light.png");theme->setCurrentText("Dark");uiResults["darkTextContrast"]=qApp->palette().color(QPalette::Text).lightness()>180;shortcuts->setChecked(false);uiResults["shortcutsDisabled"]=!table->isEnabled()&&!defaults->isEnabled();shortcuts->setChecked(true);uiResults["shortcutsReenabled"]=table->isEnabled()&&defaults->isEnabled();layout->setCurrentText("Compact");uiResults["instantCompact"]=window.findChild<QDockWidget*>("hardwareInspector")->isHidden();layout->setCurrentText("Standard");uiResults["instantStandard"]=!window.findChild<QDockWidget*>("hardwareInspector")->isHidden();dialog->grab().save(QStringLiteral(N120_SOURCE_DIR)+"/docs/settings-dark.png");auto pages=dialog->findChild<QTabWidget*>("settingsPages");pages->setCurrentIndex(5);QCoreApplication::processEvents();auto spin=dialog->findChild<QSpinBox*>("batteryPercent");QStyleOptionSpinBox opt;opt.initFrom(spin);opt.rect=spin->rect();opt.frame=true;opt.buttonSymbols=QAbstractSpinBox::UpDownArrows;opt.subControls=QStyle::SC_All;auto arrow=spin->style()->subControlRect(QStyle::CC_SpinBox,&opt,QStyle::SC_SpinBoxUp,spin);auto edit=spin->findChild<QLineEdit*>();uiResults["batteryEditorClearOfArrows"]=edit&&!edit->geometry().intersects(arrow);dialog->grab().save(QStringLiteral(N120_SOURCE_DIR)+"/docs/settings-battery.png");dialog->reject();}});window.findChild<QAction*>("settings")->trigger();});
        QTimer::singleShot(12000,&app,[&]{window.findChild<QAction*>("pause")->trigger();});
        QTimer::singleShot(12500,&app,[&]{window.findChild<QAction*>("resume")->trigger();});
    }
    if(firmwareSmoke){Configuration c;c.renode="F:/Programs/Renode/renode.exe";c.internal="C:/Users/Dotmazy/Desktop/NEW/internal to test.bin";c.external="C:/Users/Dotmazy/Desktop/NEW/external to test.bin";window.startWith(c);QTimer::singleShot(30000,&app,[&]{window.grab().save(QStringLiteral(N120_SOURCE_DIR)+"/docs/gui-smoke.png");auto state=window.lastSnapshot();state.remove("frame");state["uiChecks"]=uiResults;QFile report(QStringLiteral(N120_SOURCE_DIR)+"/docs/gui-smoke-state.json");if(report.open(QIODevice::WriteOnly))report.write(QJsonDocument(state).toJson());bool uiOk=true;for(auto value:uiResults)uiOk=uiOk&&value.toBool();app.exit(window.hasFrame()&&uiOk?0:2);});}
    return app.exec();
}
