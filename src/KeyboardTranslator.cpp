#include "KeyboardTranslator.h"
#include <QSettings>

QMap<int,int> KeyboardTranslator::defaults() {
    return {{Qt::Key_Left,0},{Qt::Key_Up,1},{Qt::Key_Down,2},{Qt::Key_Right,3},{Qt::Key_F1,4},{Qt::Key_Escape,5},{Qt::Key_Home,6},{Qt::Key_F2,8},{Qt::Key_F3,12},{Qt::Key_F4,13},{Qt::Key_Backspace,17},{Qt::Key_Delete,17},{Qt::Key_Return,52},{Qt::Key_Enter,52}};
}
KeyboardTranslator::KeyboardTranslator(QObject* parent):QObject(parent){load();}
void KeyboardTranslator::load(){QSettings s;mapping=defaults();auto saved=s.value("keyboard/mapping").toMap();if(!saved.isEmpty()){mapping.clear();for(auto it=saved.begin();it!=saved.end();++it)mapping[it.key().toInt()]=it.value().toInt();}textEnabled=s.value("keyboard/text",true).toBool();}
void KeyboardTranslator::save() const {QVariantMap values;for(auto it=mapping.begin();it!=mapping.end();++it)values[QString::number(it.key())]=it.value();QSettings s;s.setValue("keyboard/mapping",values);s.setValue("keyboard/text",textEnabled);}
QList<int> KeyboardTranslator::character(QChar c) {
    static const int alpha[]={18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,36,37,38,39,40,42,43,44,45};
    if(c.toLower()>='a'&&c.toLower()<='z'){int key=alpha[c.toLower().unicode()-'a'];return c.isUpper()?QList<int>{12,13,key}:QList<int>{13,key};}
    static const QMap<QChar,int> ordinary={{'0',48},{'1',42},{'2',43},{'3',44},{'4',36},{'5',37},{'6',38},{'7',30},{'8',31},{'9',32},{'(',33},{')',34},{'+',45},{'-',46},{'*',39},{'/',40},{'.',49},{',',22}};
    if(ordinary.contains(c))return {ordinary[c]};
    static const QMap<QChar,int> alphaSymbols={{':',14},{';',15},{'"',16},{'%',17},{' ',46},{'?',48},{'!',49},{'\\',50},{'@',51}};
    if(alphaSymbols.contains(c))return {13,alphaSymbols[c]};
    static const QMap<QChar,int> shifted={{'[',18},{']',19},{'{',20},{'}',21},{'=',27},{'<',28},{'>',29},{'_',46}};
    if(shifted.contains(c))return {12,shifted[c]};
    if(c=='\n')return {52};
    return {};
}
bool KeyboardTranslator::handle(QKeyEvent* e) {
    if(e->type()==QEvent::KeyRelease&&held.contains(e->key())){
        if(e->isAutoRepeat())return true;
        int key=mapping.value(e->key());held.remove(e->key());bool another=false;for(int pc:held)if(mapping.value(pc)==key)another=true;
        if(!another)emit physicalKey(key,false);
        return true;
    }
    if(e->modifiers().testFlag(Qt::ControlModifier)||e->modifiers().testFlag(Qt::MetaModifier))return false;
    if(mapping.contains(e->key())){
        if(e->isAutoRepeat())return true;
        int key=mapping[e->key()];if(e->type()==QEvent::KeyPress){bool another=false;for(int pc:held)if(mapping.value(pc)==key)another=true;held.insert(e->key());if(!another)emit physicalKey(key,true);}return true;
    }
    if(!textEnabled||e->text().isEmpty()||!e->text()[0].isPrint())return false;
    if(e->type()==QEvent::KeyPress){QList<int> keys;for(QChar c:e->text()){auto next=character(c);if(next.isEmpty()){emit unsupported(QString(c));return true;}keys+=next;}emit sequence(keys);}return true;
}
void KeyboardTranslator::releaseAll(){QSet<int> released;for(int pc:held)released.insert(mapping.value(pc));for(int key:released)emit physicalKey(key,false);held.clear();}
