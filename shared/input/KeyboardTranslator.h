#pragma once
#include <QKeyEvent>
#include <QMap>
#include <QObject>
#include <QSet>

class KeyboardTranslator final : public QObject {
    Q_OBJECT
  public:
    explicit KeyboardTranslator(QObject* parent = nullptr);
    static QList<int> character(QChar c);
    static QMap<int, int> defaults();
    QMap<int, int> mapping;
    bool textEnabled = true;
    bool handle(QKeyEvent* event);
    void releaseAll();

    bool holds(int key) const {
        return held.contains(key);
    }

    void load();
    void save() const;
  signals:
    void physicalKey(int index, bool pressed);
    void sequence(const QList<int>& keys);
    void unsupported(const QString& text);

  private:
    QSet<int> held;
};
