#include "MobileController.h"
#include "core/Firmware.h"
#include "input/CalculatorLayout.h"
#include "resources/CalculatorStyle.h"
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QGuiApplication>
#include <QJsonArray>
#include <QJsonDocument>
#include <QSaveFile>
#include <QStandardPaths>
#include <QtEndian>

MobileController::MobileController(QObject* parent)
    : QObject(parent), framebuffer(320, 240, QImage::Format_RGB16) {
    directory = QStandardPaths::writableLocation(QStandardPaths::AppDataLocation);
    selectedStyle =
        normalizedCalculatorStyle(settings.value("appearance/calculatorStyle", "upsilon").toString());
    QDir().mkpath(directory);
    framebuffer.fill(Qt::black);
    connect(&backend, &RemoteBackend::state, this, [this](const QString& text) {
        statusText = text;
        emit changed();
    });
    connect(&backend, &RemoteBackend::log, this, [this](const QString& text) {
        errorText = text;
        appendLog(text);
    });
    reconnectTimer.setInterval(3000);
    connect(&reconnectTimer, &QTimer::timeout, this, [this] {
        if (wantConnection && !connected() && !backend.connecting() &&
            qApp->applicationState() == Qt::ApplicationActive)
            connectHost();
    });
    reconnectTimer.start();
    connect(&backend, &RemoteBackend::connectionChanged, this, [this] {
        sentKeys.clear();
        speedClock.invalidate();
        speed = 0;
        if (!connected()) {
            fingers.clear();
            physical.clear();
            keyboard.releaseAll();
            snapshotData = {};
            framebuffer.fill(Qt::black);
            emit frameChanged();
        }
        emit heldChanged();
        emit changed();
    });
    connect(&backend, &RemoteBackend::snapshot, this, [this](QJsonObject data) {
        if (applyStartupSettings) {
            applyStartupSettings = false;
            battery(setting("battery", 100).toInt());
            cable(setting("cable", 0).toInt());
            speedTarget(setting("speed", 1).toDouble());
        }
        const auto encoded = data.take("frame").toString();
        if (!encoded.isEmpty()) {
            const auto bytes = QByteArray::fromBase64(encoded.toLatin1());
            if (bytes.size() == 320 * 240 * 2) {
                for (int y = 0; y < 240; ++y)
                    for (int x = 0; x < 320; ++x)
                        reinterpret_cast<quint16*>(framebuffer.scanLine(y))[x] =
                            qFromLittleEndian<quint16>(bytes.constData() + 2 * (y * 320 + x));
                emit frameChanged();
            }
        }
        const double virtualMs = data["virtualMs"].toDouble();
        if (speedClock.isValid() && virtualMs >= previousVirtual && speedClock.elapsed() > 0)
            speed = (virtualMs - previousVirtual) / speedClock.elapsed();
        else
            speed = 0;
        previousVirtual = virtualMs;
        speedClock.restart();
        snapshotData = data;
        emit changed();
    });
    connect(&backend, &RemoteBackend::message, this, [this](const QJsonObject& p) {
        const auto type = p["type"].toString();
        if (type == "welcome" && resumeOnForeground && qApp->applicationState() == Qt::ApplicationActive) {
            resumeOnForeground = false;
            backend.resume();
        }
        if (type == "log")
            appendLog(p["text"].toString());
        if (type == "reloaded") {
            statusText = "Firmware reloaded on Windows host";
            emit changed();
        }
        if (type == "imported") {
            statusText = "Uploaded " + p["kind"].toString() + " to Windows host";
            emit changed();
        }
        if (type == "flash") {
            const auto decoded = QByteArray::fromBase64Encoding(p["data"].toString().toLatin1(),
                                                                QByteArray::AbortOnBase64DecodingErrors);
            if (!decoded || decoded.decoded.size() != qsizetype(n120::flashCapacity)) {
                errorText = "Host returned an invalid flash backup";
                emit changed();
                return;
            }
            savedFlash = decoded.decoded;
            QSaveFile backup(directory + "/flash-backup.bin");
            if (!backup.open(QIODevice::WriteOnly) || backup.write(savedFlash) != savedFlash.size() ||
                !backup.commit()) {
                errorText = "Cannot save flash backup in app storage";
                emit changed();
                return;
            }
            emit flashReady();
        }
    });
    connect(&keyboard, &KeyboardTranslator::physicalKey, this, [this](int key, bool down) {
        if (down)
            physical.insert(key);
        else
            physical.remove(key);
        applyHeld();
    });
    connect(&keyboard, &KeyboardTranslator::sequence, this, [this](const QList<int>& sequence) {
        QStringList values;
        for (int key : sequence)
            values << QString::number(key);
        backend.command("sequence " + values.join(',').toUtf8());
    });
    qApp->installEventFilter(this);
    connect(qApp, &QGuiApplication::applicationStateChanged, this, [this](Qt::ApplicationState state) {
        if (state != Qt::ApplicationActive) {
            releaseTouches();
            keyboard.releaseAll();
            if (setting("pauseBackground", true).toBool() && connected()) {
                resumeOnForeground = resumeOnForeground || (running() && !paused());
                backend.pause();
            }
        } else if (resumeOnForeground && connected()) {
            resumeOnForeground = false;
            backend.resume();
        }
    });
}

QVariant MobileController::setting(const QString& name, const QVariant& fallback) const {
    return settings.value(name, fallback);
}

void MobileController::setSetting(const QString& name, const QVariant& value) {
    settings.setValue(name, value);
    settings.sync();
}

void MobileController::appendLog(const QString& text) {
    logLines << text.left(8192);
    while (logLines.size() > 250)
        logLines.removeFirst();
    emit changed();
}

void MobileController::clearError() {
    errorText.clear();
    emit changed();
}

void MobileController::connectHost() {
    wantConnection = true;
    clearError();
    backend.connectHost(setting("host").toString(), quint16(setting("port", 7420).toUInt()),
                        setting("pin").toString(), setting("token").toString());
}

void MobileController::disconnectHost() {
    wantConnection = false;
    resumeOnForeground = false;
    releaseTouches();
    keyboard.releaseAll();
    backend.disconnectHost();
    statusText = "Offline · Windows host required";
    emit changed();
}

void MobileController::action(const QString& name) {
    clearError();
    if (name == "start") {
        applyStartupSettings = true;
        Configuration c;
        c.lcdId = setting("lcd", 0x4E4101).toUInt();
        backend.start(c);
    } else if (name == "stop") {
        releaseTouches();
        keyboard.releaseAll();
        backend.stop();
    } else if (name == "pause") {
        releaseTouches();
        backend.pause();
    } else if (name == "resume")
        backend.resume();
    else if (name == "reset") {
        releaseTouches();
        backend.reset();
    } else if (name == "reload" || name == "restart") {
        if (name == "restart")
            applyStartupSettings = true;
        releaseTouches();
        keyboard.releaseAll();
        backend.send({{"type", name}});
    }
}

void MobileController::battery(int p) {
    setSetting("battery", p);
    backend.setBattery(p);
}

void MobileController::cable(int c) {
    setSetting("cable", c);
    backend.setCable(c);
}

void MobileController::speedTarget(double s) {
    setSetting("speed", s);
    backend.command("speed " + QByteArray::number(s, 'f', 2));
}

QVariantList MobileController::keys() const {
    QVariantList result;
    for (const auto& k : calculatorKeys())
        result << QVariantMap{{"index", k.index},  {"name", k.name},        {"x", k.bounds.x()},
                              {"y", k.bounds.y()}, {"w", k.bounds.width()}, {"h", k.bounds.height()}};
    return result;
}

QVariantList MobileController::held() const {
    QVariantList result;
    for (int k : sentKeys)
        result << k;
    return result;
}

QVariantList MobileController::styleOptions() const {
    QVariantList result;
    for (const auto& style : calculatorStyles())
        result << QVariantMap{{"id", style.id}, {"name", style.name}};
    return result;
}

QString MobileController::calculatorSkin() const {
    return "qrc" + calculatorStyleResource(selectedStyle);
}

void MobileController::setCalculatorStyle(const QString& id) {
    const auto selected = normalizedCalculatorStyle(id);
    if (selectedStyle == selected)
        return;
    releaseTouches();
    keyboard.releaseAll();
    selectedStyle = selected;
    setSetting("appearance/calculatorStyle", selected);
    emit calculatorStyleChanged();
}

QColor MobileController::led() const {
    const auto rgb = snapshotData["rgb"].toArray();
    return QColor::fromRgbF(qBound(0.0, rgb[0].toDouble(), 1.0), qBound(0.0, rgb[1].toDouble(), 1.0),
                            qBound(0.0, rgb[2].toDouble(), 1.0));
}

QString MobileController::diagnostics() const {
    return QString::fromUtf8(QJsonDocument(snapshotData).toJson(QJsonDocument::Indented));
}

void MobileController::touch(int id, double x, double y, bool down) {
    int key = -1;
    if (down)
        key = calculatorHitKey(QPointF(x, y));
    if (key < 0)
        fingers.remove(id);
    else
        fingers[id] = key;
    applyHeld();
}

void MobileController::applyHeld() {
    QSet<int> desired = physical;
    for (int key : fingers)
        desired.insert(key);
    for (int k : sentKeys)
        if (!desired.contains(k))
            backend.setKey(k, false);
    for (int k : desired)
        if (!sentKeys.contains(k))
            backend.setKey(k, true);
    sentKeys = desired;
    emit heldChanged();
}

void MobileController::releaseTouches() {
    fingers.clear();
    applyHeld();
}

void MobileController::tapKey(int index) {
    if (index >= 0 && index < 54)
        backend.command("sequence " + QByteArray::number(index));
}

bool MobileController::eventFilter(QObject*, QEvent* e) {
    if (e->type() == QEvent::ApplicationDeactivate) {
        releaseTouches();
        keyboard.releaseAll();
    }
    // Text fields retain ordinary Android/physical keyboard input.
    if ((e->type() == QEvent::KeyPress || e->type() == QEvent::KeyRelease) && qApp->focusObject() &&
        qApp->focusObject()->objectName() == "calculatorFocus")
        return keyboard.handle(static_cast<QKeyEvent*>(e));
    return false;
}

QString MobileController::imagePath(const QString& kind) const {
    if (kind != "internal" && kind != "slotA" && kind != "slotB" && kind != "flash")
        return {};
    return directory + '/' + kind + ".bin";
}

QString MobileController::files() const {
    QStringList result;
    for (const auto& kind : {"internal", "slotA", "slotB", "flash"}) {
        QFileInfo f(imagePath(kind));
        if (f.exists())
            result << QString("%1 · %2 bytes").arg(kind).arg(f.size());
    }
    return result.isEmpty() ? "No firmware imported. Firmware is never bundled." : result.join('\n');
}

void MobileController::importFile(const QUrl& url, const QString& kind) {
    if (imagePath(kind).isEmpty())
        return;
    // Qt's Android file engine opens content:// URIs granted by the native SAF picker.
    QFile source(url.isLocalFile() ? url.toLocalFile() : url.toString());
    if (!source.open(QIODevice::ReadOnly)) {
        errorText = "Cannot open selected file: " + source.errorString();
        emit changed();
        return;
    }
    const qint64 limit = kind == "internal" ? n120::internalCapacity
                         : kind == "slotB"  ? n120::slotCapacity
                                            : n120::flashCapacity;
    auto bytes = source.read(limit + 1);
    if (bytes.isEmpty() || bytes.size() > limit || !source.atEnd()) {
        errorText = "Image is empty or exceeds the selected mapping";
        emit changed();
        return;
    }
    const auto validation =
        kind == "internal"
            ? n120::validateInternal(
                  {reinterpret_cast<const std::uint8_t*>(bytes.constData()), std::size_t(bytes.size())})
            : n120::validateExternal(std::size_t(bytes.size()), kind == "slotB" ? 0x400000 : 0);
    if (!validation.empty()) {
        errorText = QString::fromStdString(validation);
        emit changed();
        return;
    }
    QSaveFile out(imagePath(kind));
    if (!out.open(QIODevice::WriteOnly) || out.write(bytes) != bytes.size() || !out.commit())
        errorText = "Cannot store firmware in app storage";
    else {
        errorText.clear();
        statusText = "Imported " + kind + " into phone storage. Upload while stopped to use it on the host.";
    }
    emit changed();
}

void MobileController::upload(const QString& kind) {
    if (!connected() || running()) {
        errorText = "Connect and stop emulation before uploading";
        emit changed();
        return;
    }
    QFile image(imagePath(kind));
    if (!image.open(QIODevice::ReadOnly)) {
        errorText = "Import this image first";
        emit changed();
        return;
    }
    backend.send({{"type", "import"},
                  {"kind", kind},
                  {"data", QString::fromLatin1(image.read(n120::flashCapacity + 1).toBase64())}});
    statusText = "Uploading " + kind + "…";
    emit changed();
}

void MobileController::exportFlash() {
    backend.send({{"type", "exportFlash"}});
}

QUrl MobileController::exportSuggestion(const QString& kind) const {
    const auto name = kind == "flash"        ? "n120-flash.bin"
                      : kind == "screenshot" ? "n120-lcd.png"
                                             : "n120-diagnostics.txt";
    return QUrl::fromLocalFile(
        QDir(QStandardPaths::writableLocation(QStandardPaths::DocumentsLocation)).filePath(name));
}

void MobileController::exportFile(const QUrl& url, const QString& kind) {
    QFile out(url.isLocalFile() ? url.toLocalFile() : url.toString());
    if (!out.open(QIODevice::WriteOnly)) {
        errorText = "Cannot write selected export destination";
        emit changed();
        return;
    }
    QByteArray bytes;
    if (kind == "diagnostics")
        bytes = ("Execution: REMOTE (Windows host)\n" + diagnostics() + "\n" + logs()).toUtf8();
    else if (kind == "flash")
        bytes = savedFlash;
    else if (kind == "screenshot") {
        if (!framebuffer.save(&out, "PNG"))
            errorText = "Cannot save LCD screenshot";
        emit changed();
        return;
    }
    if (bytes.isEmpty() || out.write(bytes) != bytes.size() || !out.flush())
        errorText = "Export failed";
    else
        statusText = "Export saved";
    emit changed();
}
