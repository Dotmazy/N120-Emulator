#pragma once
#include <QFileInfo>
#include <QObject>
#include <QProcess>
#include <QRegularExpression>
#include <QStandardPaths>
#include <QTimer>

// Only attaches the emulator's fixed loopback bus. Never detaches arbitrary ports.
class WindowsUsb final : public QObject {
    Q_OBJECT
  public:
    explicit WindowsUsb(QObject* parent = nullptr) : QObject(parent) {
        executable = QStandardPaths::findExecutable("usbip.exe");
        if (executable.isEmpty() && QFileInfo::exists("C:/Program Files/USBip/usbip.exe"))
            executable = "C:/Program Files/USBip/usbip.exe";
        connect(
            &process, qOverload<int, QProcess::ExitStatus>(&QProcess::finished), this,
            [this](int code, QProcess::ExitStatus) {
                const auto output = QString::fromLocal8Bit(process.readAllStandardOutput()) +
                                    QString::fromLocal8Bit(process.readAllStandardError());
                if (operation == "verify-detach") {
                    if (code == 0 && emulatorPort(output) == ownedPort && ownedPort != 0)
                        run("detach", {"detach", "-p", QString::number(ownedPort)});
                    else {
                        if (code == 0)
                            ownedPort = 0;
                        emit status(
                            "USB/IP detach skipped: the owned loopback attachment could not be verified.");
                    }
                    return;
                }
                if (operation == "preflight") {
                    if (emulatorPort(output) != 0) {
                        emit status(output.trimmed() +
                                    "\nAlready imported outside this application's attachment workflow.");
                        return;
                    }
                    if (code != 0) {
                        emit status(output.trimmed());
                        return;
                    }
                    claimNewPort = true;
                    QTimer::singleShot(0, this,
                                       [this] { run("attach", {"attach", "-r", "127.0.0.1", "-b", "1-1"}); });
                    return;
                }
                if (operation == "attach" && code == 0) {
                    auto match = QRegularExpression("attached to port (\\d+)").match(output);
                    if (match.hasMatch())
                        ownedPort = match.captured(1).toInt();
                }
                if (operation == "detach" && code == 0)
                    ownedPort = 0;
                emit message(operation + ": " + output.trimmed());
                if (operation == "inspect") {
                    if (claimNewPort) {
                        ownedPort = emulatorPort(output);
                        claimNewPort = false;
                    }
                    emit status(output.trimmed());
                } else {
                    emit status(output.trimmed());
                    if (operation == "attach")
                        QTimer::singleShot(1200, this, &WindowsUsb::inspect);
                }
                if (detachPending) {
                    detachPending = false;
                    detach();
                }
            });
        connect(&process, &QProcess::errorOccurred, this, [this] { emit status(process.errorString()); });
    }

    ~WindowsUsb() override {
        disconnect(&process, nullptr, this, nullptr);
        disconnect(this, nullptr, nullptr, nullptr);
        if (process.state() != QProcess::NotRunning) {
            process.kill();
            process.waitForFinished(1000);
        }
        if (ownedPort) {
            QProcess cleanup;
            cleanup.start(executable, {"port"});
            if (cleanup.waitForFinished(2000) && cleanup.exitCode() == 0 &&
                emulatorPort(QString::fromLocal8Bit(cleanup.readAllStandardOutput())) == ownedPort) {
                cleanup.start(executable, {"detach", "-p", QString::number(ownedPort)});
                cleanup.waitForFinished(2000);
            }
        }
    }

    bool available() const {
        return !executable.isEmpty();
    }

    void attach() {
        if (!available()) {
            emit status("USB/IP client missing. Install the signed prerequisite in docs/usb.md.");
            return;
        }
        if (ownedPort) {
            inspect();
            return;
        }
        run("preflight", {"port"});
    }

    void detach() {
        if (process.state() != QProcess::NotRunning) {
            detachPending = true;
            return;
        }
        if (ownedPort)
            run("verify-detach", {"port"});
    }

    void inspect() {
        if (!available()) {
            emit status("USB/IP importing client is not installed");
            return;
        }
        if (process.state() != QProcess::NotRunning)
            return;
        // Read-only OS inspection. Imported port and PnP status are reported
        // separately; the firmware's descriptor probe is never an OS success flag.
        operation = "inspect";
        process.start(
            "powershell.exe",
            {"-NoProfile", "-Command",
             QString(
                 "& '%1' port; Get-PnpDevice -PresentOnly | Where-Object {$_.InstanceId -match "
                 "'VID_0483&PID_A291|VID_0483&PID_DF11'} | ForEach-Object {$driver=(Get-PnpDeviceProperty "
                 "-InstanceId $_.InstanceId -KeyName DEVPKEY_Device_Service).Data; Write-Output ($_.Status+' "
                 "| '+$_.FriendlyName+' | '+$driver+' | '+$_.InstanceId)}")
                 .arg(QString(executable).replace("'", "''"))});
    }

  signals:
    void status(const QString&);
    void message(const QString&);

  private:
    static int emulatorPort(const QString& output) {
        const auto match =
            QRegularExpression(
                "Port\\s+(\\d+):(?:(?!Port\\s+\\d+:)[\\s\\S])*?usbip://127\\.0\\.0\\.1:3240/1-1")
                .match(output);
        return match.hasMatch() ? match.captured(1).toInt() : 0;
    }

    void run(const QString& name, const QStringList& args) {
        if (process.state() != QProcess::NotRunning)
            return;
        operation = name;
        process.start(executable, args);
    }

    QProcess process;
    QString executable, operation;
    int ownedPort = 0;
    bool detachPending = false, claimNewPort = false;
};
