#pragma once
#include <QJsonObject>
#include <QObject>

struct Configuration {
    QString renode, internal, external, persistent;
    quint32 lcdId = 0x4E4101;
    quint32 externalOffset = 0;
    bool boot0 = false;
    QString systemRom;
};

// Frontends depend on this event-driven contract, never on a host process.
// Snapshots carry real backend values (including virtualMs and instructions).
class EmulatorBackend : public QObject {
    Q_OBJECT
  public:
    using QObject::QObject;
    virtual void start(const Configuration&) = 0;
    virtual void stop() = 0;
    virtual void command(const QByteArray&) = 0;
    virtual bool connected() const = 0;
    virtual bool running() const = 0;

    void pause() {
        command("pause");
    }

    void resume() {
        command("resume");
    }

    void reset() {
        command("reset");
    }

    void setKey(int index, bool pressed) {
        if (index >= 0 && index < 54)
            command("key " + QByteArray::number(index / 6) + ' ' + QByteArray::number(index % 6) +
                    (pressed ? " 1" : " 0"));
    }

    void setBattery(int percent) {
        command("battery " + QByteArray::number(qBound(0, percent, 100)));
    }

    void setCable(int mode) {
        if (mode >= 0 && mode <= 2)
            command("cable " + QByteArray::number(mode));
    }

  signals:
    void snapshot(const QJsonObject&);
    void log(const QString&);
    void state(const QString&);
    void response(const QJsonObject&);
};
