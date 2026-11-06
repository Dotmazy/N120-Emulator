#pragma once
#include "CalculatorWidget.h"
#include "KeyboardTranslator.h"
#include "RenodeManager.h"
#include "WindowsUsb.h"
#include <QElapsedTimer>
#include <QJsonObject>
#include <QMainWindow>
#include <QSet>
QIcon studioIcon(const QString&, QColor);
class QLabel;
class QDockWidget;
class QTabWidget;
class QSlider;
class QComboBox;
class QToolBar;
class LogViewer;

class MainWindow final : public QMainWindow {
    Q_OBJECT
  public:
    explicit MainWindow(bool uiSmoke = false);
    ~MainWindow() override;

    QString hostUsbStatus() const {
        return windowsUsbStatus;
    }

    QJsonObject lastSnapshot() const {
        return hardware;
    }

    void startWith(const Configuration&);

    bool hasFrame() const {
        return firmwarePixels > 300000 && externalExecuting;
    }

    CalculatorWidget* calculator() const {
        return surface;
    }

  protected:
    bool eventFilter(QObject*, QEvent*) override;
    void closeEvent(QCloseEvent*) override;

  private:
    void configure();
    void buildMenus();
    void updateHardware(const QJsonObject&);
    void key(int, bool, bool mouse = false);
    void releaseKeys();
    void power();
    void restart();
    void setLayoutMode(const QString&);
    void setCompact(bool);
    void applyPreferences();
    void settings(int page = 0);
    void saveFlash();
    void screenshot(bool full);
    void exportReport();
    void console();
    void recentFiles();
    void updateActions();
    void speedSettings();
    void applyZoom();
    void updatePowerCard();
    WindowsUsb windowsUsb;
    QString windowsUsbStatus = "Windows attachment not checked";
    RenodeManager engine;
    KeyboardTranslator keyboard;
    Configuration config;
    CalculatorWidget* surface;
    QDockWidget* sidebar;
    QTabWidget* tabs;
    QToolBar* toolbar;
    QLabel *standardInfo, *boardInfo, *overview, *cpuInfo, *displayInfo, *powerInfo, *usbInfo, *flashInfo,
        *liveStatus;
    QSlider* battery;
    QComboBox* cable;
    LogViewer* logs;
    QMenu* recentMenu;
    QMap<QString, QAction*> actions;
    QSet<int> mouseHeld, pcHeld;
    QJsonObject hardware;
    QSet<QString> reportedErrors;
    QString layoutName = "Standard";
    bool pendingRestart = false;
    quint64 firmwarePixels = 0, lastPixels = 0;
    bool externalExecuting = false, compact = false;
    double speed = 0, lastVirtual = -1, zoom = 1;
    QElapsedTimer sampleClock;
};
