#pragma once
#include <QObject>
#include <QMap>
#include <QSet>
#include <QKeyEvent>

class KeyboardTranslator final : public QObject {
    Q_OBJECT
public:
    explicit KeyboardTranslator(QObject* parent=nullptr);
    static QList<int> character(QChar c);
    static QMap<int,int> defaults();
    QMap<int,int> mapping;
    bool textEnabled=true;
    bool handle(QKeyEvent* event);
    void releaseAll();
    void load();
    void save() const;
signals:
    void physicalKey(int index,bool pressed);
    void sequence(const QList<int>& keys);
    void unsupported(const QString& text);
private:
    QSet<int> held;
};
