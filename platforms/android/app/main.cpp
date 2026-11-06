#include "LcdItem.h"
#include "MobileController.h"
#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickStyle>
#include <QTimer>

int main(int argc, char** argv) {
#ifdef Q_OS_ANDROID
    // GLES is supported on the Android 9 baseline and avoids emulator Vulkan driver corruption.
    qputenv("QSG_RHI_BACKEND", "opengl");
#endif
    QGuiApplication app(argc, argv);
    app.setOrganizationName("N120Emulator");
    app.setApplicationName("N120Mobile");
    QQuickStyle::setStyle("Material");
    qmlRegisterType<LcdItem>("N120.Display", 1, 0, "LcdItem");
    qmlRegisterUncreatableType<MobileController>("N120.Display", 1, 0, "MobileController",
                                                 "Owned by application");
    MobileController controller;
    QQmlApplicationEngine engine;
    engine.rootContext()->setContextProperty("emulator", &controller);
    QObject::connect(
        &engine, &QQmlApplicationEngine::objectCreationFailed, &app, [] { QCoreApplication::exit(2); },
        Qt::QueuedConnection);
    engine.loadFromModule("N120.Mobile", "Main");
    if (app.arguments().contains("--ui-smoke"))
        QTimer::singleShot(2000, &app, &QCoreApplication::quit);
    return app.exec();
}
