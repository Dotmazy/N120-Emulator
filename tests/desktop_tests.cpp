#include "CalculatorWidget.h"
#include "KeyboardTranslator.h"
#include <QtTest>
#include <QTemporaryDir>
#include <QSettings>
class DesktopTests:public QObject {
    Q_OBJECT
private slots:
    void hitboxesStayAligned(){
        CalculatorWidget w;w.show();QSignalSpy spy(&w,&CalculatorWidget::physicalKey);
        QCOMPARE(CalculatorWidget::keys().size(),46);
        for(QSize size:{QSize(350,700),QSize(750,850),QSize(500,1100)}){
            w.resize(size);QCoreApplication::processEvents();auto r=w.imageRect();
            for(const auto& key:CalculatorWidget::keys()){
                QPointF c=key.bounds.center();QPoint p(qRound(r.x()+c.x()*r.width()/1160),qRound(r.y()+c.y()*r.height()/2221));
                QCOMPARE(w.hitKey(p),key.index);QTest::mousePress(&w,Qt::LeftButton,Qt::NoModifier,p);QTest::mouseRelease(&w,Qt::LeftButton,Qt::NoModifier,p);
                QCOMPARE(spy.takeFirst()[0].toInt(),key.index);QCOMPARE(spy.takeFirst()[1].toBool(),false);
            }
            QCOMPARE(w.hitKey(QPointF(0,0)),-1);
        }
    }
    void backlightDoesNotMutateFramebuffer(){
        CalculatorWidget w;QByteArray pixels(320*240*2,'\xFF');w.setFrame(pixels);QImage before=w.lcdImage();
        w.setDim(true);w.setHardware({.25f,.75f,.5f},0);w.resize(450,900);w.show();auto dim=w.grab();
        QCOMPARE(before,w.lcdImage());w.setDim(false);auto bright=w.grab();QVERIFY(dim.toImage()!=bright.toImage());QCOMPARE(before,w.lcdImage());
    }
    void textAndRawKeys(){
        KeyboardTranslator t;QSignalSpy sequences(&t,&KeyboardTranslator::sequence),physical(&t,&KeyboardTranslator::physicalKey);
        QKeyEvent event(QEvent::KeyPress,Qt::Key_P,Qt::NoModifier,"p");QVERIFY(t.handle(&event));QCOMPARE(sequences.count(),1);QCOMPARE(sequences.takeFirst()[0].value<QList<int>>(),QList<int>({13,33}));
        QCOMPARE(KeyboardTranslator::character('H'),QList<int>({12,13,25}));QCOMPARE(KeyboardTranslator::character('"'),QList<int>({13,16}));
        QKeyEvent down(QEvent::KeyPress,Qt::Key_F2,Qt::NoModifier),up(QEvent::KeyRelease,Qt::Key_F2,Qt::NoModifier);QVERIFY(t.handle(&down));QVERIFY(t.handle(&up));QCOMPARE(physical.count(),2);QCOMPARE(physical[0][0].toInt(),8);QCOMPARE(physical[0][1].toBool(),true);QCOMPARE(physical[1][1].toBool(),false);
        QKeyEvent repeat(QEvent::KeyPress,Qt::Key_P,Qt::NoModifier,"p",true);QVERIFY(t.handle(&repeat));QCOMPARE(sequences.count(),1);
    }
    void mappingsPersist(){KeyboardTranslator t;t.mapping={{Qt::Key_F10,8}};t.textEnabled=false;t.save();KeyboardTranslator restored;QCOMPARE(restored.mapping,t.mapping);QVERIFY(!restored.textEnabled);restored.mapping=KeyboardTranslator::defaults();restored.textEnabled=true;restored.save();}
    void overlappingMappingsAndModifiedRelease(){
        KeyboardTranslator t;QSignalSpy keys(&t,&KeyboardTranslator::physicalKey);
        QKeyEvent a(QEvent::KeyPress,Qt::Key_Return,Qt::NoModifier),b(QEvent::KeyPress,Qt::Key_Enter,Qt::NoModifier),releaseA(QEvent::KeyRelease,Qt::Key_Return,Qt::ControlModifier),releaseB(QEvent::KeyRelease,Qt::Key_Enter,Qt::NoModifier);
        t.handle(&a);t.handle(&b);QCOMPARE(keys.count(),1);t.handle(&releaseA);QCOMPARE(keys.count(),1);t.handle(&releaseB);QCOMPARE(keys.count(),2);QCOMPARE(keys[1][1].toBool(),false);
    }
};
int main(int argc,char** argv){QApplication app(argc,argv);QTemporaryDir settings;QSettings::setDefaultFormat(QSettings::IniFormat);QSettings::setPath(QSettings::IniFormat,QSettings::UserScope,settings.path());app.setOrganizationName("N120DesktopTests");app.setApplicationName("Tests");DesktopTests tests;return QTest::qExec(&tests,argc,argv);}
#include "desktop_tests.moc"
