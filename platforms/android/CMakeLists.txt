find_package(Qt6 REQUIRED COMPONENTS Quick QuickControls2 Qml)
qt_standard_project_setup(REQUIRES 6.5 SUPPORTS_UP_TO 6.11)
qt_add_executable(n120_mobile app/main.cpp app/MobileController.cpp app/MobileController.h ui/LcdItem.cpp ui/LcdItem.h)
target_link_libraries(n120_mobile PRIVATE n120_remote n120_core n120_input Qt6::Quick Qt6::QuickControls2)
target_include_directories(n120_mobile PRIVATE app ui)
qt_add_qml_module(n120_mobile URI N120.Mobile VERSION 1.0 QML_FILES ui/Main.qml)
n120_add_calculator_assets(n120_mobile mobile_assets)
qt_add_resources(n120_mobile mobile_notices PREFIX "/" BASE "${PROJECT_SOURCE_DIR}" FILES "${PROJECT_SOURCE_DIR}/THIRD_PARTY_NOTICES.md")
if(ANDROID)
    set(N120_ANDROID_OPENSSL_ROOT "" CACHE PATH "OpenSSL 3.5.9 Android install root (scripts/prepare-openssl-android.ps1)")
    set(openssl_libs)
    foreach(library crypto ssl)
        set(openssl_library "${N120_ANDROID_OPENSSL_ROOT}/${CMAKE_ANDROID_ARCH_ABI}/lib/lib${library}_3.so")
        if(NOT EXISTS "${openssl_library}")
            message(FATAL_ERROR "Missing ${openssl_library}. Run scripts/prepare-openssl-android.ps1 and set N120_ANDROID_OPENSSL_ROOT.")
        endif()
        list(APPEND openssl_libs "${openssl_library}")
    endforeach()
    set_property(TARGET n120_mobile APPEND PROPERTY QT_ANDROID_EXTRA_LIBS ${openssl_libs})
    qt_add_resources(n120_mobile openssl_license PREFIX "/licenses" BASE "${N120_ANDROID_OPENSSL_ROOT}"
        FILES "${N120_ANDROID_OPENSSL_ROOT}/LICENSE.txt")
endif()
set_target_properties(n120_mobile PROPERTIES
    QT_ANDROID_PACKAGE_SOURCE_DIR "${CMAKE_CURRENT_SOURCE_DIR}/android"
    QT_ANDROID_PACKAGE_NAME "org.n120.emulator"
    QT_ANDROID_APP_NAME "N120 Studio"
    QT_ANDROID_VERSION_CODE 4
    QT_ANDROID_VERSION_NAME "0.4.0"
    QT_ANDROID_MIN_SDK_VERSION 28
    QT_ANDROID_TARGET_SDK_VERSION 36)
if(BUILD_TESTING AND NOT ANDROID)
    qt_add_executable(n120_mobile_tests "${PROJECT_SOURCE_DIR}/tests/mobile_tests.cpp" app/MobileController.cpp app/MobileController.h ui/LcdItem.cpp ui/LcdItem.h)
    target_include_directories(n120_mobile_tests PRIVATE app ui)
    target_compile_definitions(n120_mobile_tests PRIVATE N120_MOBILE_QML_SOURCE="${CMAKE_CURRENT_SOURCE_DIR}/ui/Main.qml")
    target_link_libraries(n120_mobile_tests PRIVATE n120_core n120_remote n120_input Qt6::Quick Qt6::QuickControls2 Qt6::Test)
    n120_add_calculator_assets(n120_mobile_tests mobile_test_assets)
    add_test(NAME mobile_contracts COMMAND n120_mobile_tests)
    add_test(NAME mobile_qml_startup COMMAND n120_mobile --ui-smoke)
    set_tests_properties(mobile_contracts mobile_qml_startup PROPERTIES ENVIRONMENT "QT_QPA_PLATFORM=offscreen" TIMEOUT 15)
endif()
