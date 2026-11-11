#pragma once
#include "RemoteBackend.h"
#include "input/KeyboardTranslator.h"
#include <QColor>
#include <QElapsedTimer>
#include <QImage>
#include <QMap>
#include <QSettings>
#include <QUrl>
#include <QVariantList>

class MobileController final : public QObject {
    Q_OBJECT
    Q_PROPERTY(bool connected READ connected NOTIFY changed)
    Q_PROPERTY(bool running READ running NOTIFY changed)
    Q_PROPERTY(bool paused READ paused NOTIFY changed)
    Q_PROPERTY(QString status READ status NOTIFY changed)
    Q_PROPERTY(QString error READ error NOTIFY changed)
    Q_PROPERTY(QString diagnostics READ diagnostics NOTIFY changed)
    Q_PROPERTY(QString logs READ logs NOTIFY changed)
    Q_PROPERTY(QVariantList keys READ keys CONSTANT)
    Q_PROPERTY(QVariantList held READ held NOTIFY heldChanged)
    Q_PROPERTY(QVariantList calculatorStyles READ styleOptions CONSTANT)
    Q_PROPERTY(
        QString calculatorStyle READ calculatorStyle WRITE setCalculatorStyle NOTIFY calculatorStyleChanged)
    Q_PROPERTY(QString calculatorSkin READ calculatorSkin NOTIFY calculatorStyleChanged)
    Q_PROPERTY(QColor led READ led NOTIFY changed)
    Q_PROPERTY(double backlight READ backlight NOTIFY changed)
    Q_PROPERTY(double actualSpeed READ actualSpeed NOTIFY changed)
    Q_PROPERTY(QVariantMap hardware READ hardware NOTIFY changed)
    Q_PROPERTY(QString files READ files NOTIFY changed)
  public:
    explicit MobileController(QObject* parent = nullptr);

    bool connected() const {
        return backend.connected();
    }

    bool running() const {
        return backend.running();
    }

    bool paused() const {
        return snapshotData["paused"].toBool();
    }

    QString status() const {
        return statusText;
    }

    QString error() const {
        return errorText;
    }

    QString diagnostics() const;

    QString logs() const {
        return logLines.join('\n');
    }

    QVariantList keys() const;
    QVariantList held() const;
    QVariantList styleOptions() const;

    QString calculatorStyle() const {
        return selectedStyle;
    }

    QString calculatorSkin() const;
    void setCalculatorStyle(const QString& id);
    QColor led() const;

    double backlight() const {
        return snapshotData["backlight"].toDouble();
    }

    double actualSpeed() const {
        return speed;
    }

    QVariantMap hardware() const {
        return snapshotData.toVariantMap();
    }

    QString files() const;

    const QImage& frame() const {
        return framebuffer;
    }

    Q_INVOKABLE QVariant setting(const QString& name, const QVariant& fallback = {}) const;
    Q_INVOKABLE void setSetting(const QString& name, const QVariant& value);
    Q_INVOKABLE void connectHost();
    Q_INVOKABLE void disconnectHost();
    Q_INVOKABLE void action(const QString& name);
    Q_INVOKABLE void battery(int value);
    Q_INVOKABLE void cable(int value);
    Q_INVOKABLE void speedTarget(double value);
    Q_INVOKABLE void touch(int id, double x, double y, bool pressed);
    Q_INVOKABLE void releaseTouches();
    Q_INVOKABLE void tapKey(int index);
    Q_INVOKABLE void importFile(const QUrl& url, const QString& kind);
    Q_INVOKABLE void upload(const QString& kind);
    Q_INVOKABLE void exportFile(const QUrl& url, const QString& kind);
    Q_INVOKABLE QUrl exportSuggestion(const QString& kind) const;
    Q_INVOKABLE void exportFlash();
    Q_INVOKABLE void clearError();
    bool eventFilter(QObject*, QEvent*) override;
  signals:
    void changed();
    void heldChanged();
    void calculatorStyleChanged();
    void frameChanged();
    void flashReady();

  private:
    void applyHeld();
    void appendLog(const QString& text);
    QString imagePath(const QString& kind) const;
    RemoteBackend backend;
    KeyboardTranslator keyboard;
    QSettings settings;
    QString selectedStyle = "upsilon";
    QString directory, statusText = "Offline · connect a Windows host", errorText;
    QStringList logLines;
    QJsonObject snapshotData;
    QImage framebuffer;
    QMap<int, int> fingers;
    QSet<int> physical, sentKeys;
    QElapsedTimer speedClock;
    double previousVirtual = 0, speed = 0;
    bool resumeOnForeground = false;
    bool applyStartupSettings = false;
    bool wantConnection = false;
    QTimer reconnectTimer;
    QByteArray savedFlash;
};
