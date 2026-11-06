find_package(Qt6 REQUIRED COMPONENTS Widgets)
qt_add_executable(n120 WIN32 "${PROJECT_SOURCE_DIR}/src/main.cpp" "${PROJECT_SOURCE_DIR}/src/MainWindow.cpp" "${PROJECT_SOURCE_DIR}/src/MainWindowActions.cpp" "${PROJECT_SOURCE_DIR}/src/MainWindow.h" "${PROJECT_SOURCE_DIR}/src/StartupDialog.cpp" "${PROJECT_SOURCE_DIR}/src/SettingsDialog.cpp" "${PROJECT_SOURCE_DIR}/src/CalculatorWidget.cpp" "${PROJECT_SOURCE_DIR}/src/CalculatorWidget.h"  "${PROJECT_SOURCE_DIR}/src/LogViewer.cpp" "${PROJECT_SOURCE_DIR}/src/LogViewer.h" "${PROJECT_SOURCE_DIR}/src/WindowsUsb.h")
qt_add_resources(n120 calculator_assets PREFIX "/" BASE "${PROJECT_SOURCE_DIR}" FILES "${PROJECT_SOURCE_DIR}/assets/calculator.png")
set_target_properties(n120 PROPERTIES OUTPUT_NAME "n120-studio" RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}")
target_link_libraries(n120 PRIVATE Qt6::Widgets Qt6::Network n120_renode n120_input)
if(MSVC)
  target_compile_options(n120 PRIVATE /W4)
else()
  target_compile_options(n120 PRIVATE -Wall -Wextra -Wpedantic)
endif()
add_dependencies(n120 n120_resources)
if(BUILD_TESTING)
 find_package(Qt6 REQUIRED COMPONENTS Test)
 qt_add_executable(n120_desktop_tests "${PROJECT_SOURCE_DIR}/tests/desktop_tests.cpp" "${PROJECT_SOURCE_DIR}/src/CalculatorWidget.cpp" "${PROJECT_SOURCE_DIR}/src/CalculatorWidget.h" )
 target_include_directories(n120_desktop_tests PRIVATE "${PROJECT_SOURCE_DIR}/src")
 target_link_libraries(n120_desktop_tests PRIVATE Qt6::Widgets Qt6::Test n120_input)
 qt_add_resources(n120_desktop_tests test_assets PREFIX "/" BASE "${PROJECT_SOURCE_DIR}" FILES "${PROJECT_SOURCE_DIR}/assets/calculator.png")
 qt_add_executable(n120_manager_tests "${PROJECT_SOURCE_DIR}/tests/manager_tests.cpp")
 target_include_directories(n120_manager_tests PRIVATE "${PROJECT_SOURCE_DIR}/src")
 target_link_libraries(n120_manager_tests PRIVATE n120_renode Qt6::Test)
 set_target_properties(n120_desktop_tests n120_manager_tests PROPERTIES RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}")
 add_test(NAME manager_validation COMMAND n120_manager_tests)
 add_test(NAME desktop_controls COMMAND n120_desktop_tests)
 add_test(NAME qt_startup COMMAND n120 --ui-smoke)
 set_tests_properties(manager_validation desktop_controls qt_startup PROPERTIES TIMEOUT 15)
 set_tests_properties(desktop_controls qt_startup PROPERTIES ENVIRONMENT "QT_QPA_PLATFORM=offscreen")
endif()

# Stage only public runtime inputs; Renode, drivers and firmware remain external.
install(TARGETS n120 RUNTIME DESTINATION .)
install(DIRECTORY "${PROJECT_SOURCE_DIR}/renode/" DESTINATION renode FILES_MATCHING PATTERN "*.cs" PATTERN "*.repl")
install(FILES "${PROJECT_SOURCE_DIR}/README.md" "${PROJECT_SOURCE_DIR}/THIRD_PARTY_NOTICES.md" DESTINATION .)
set(N120_PUBLIC_DOCS
    "${PROJECT_SOURCE_DIR}/docs/architecture.md"
    "${PROJECT_SOURCE_DIR}/docs/building.md"
    "${PROJECT_SOURCE_DIR}/docs/usb.md"
    "${PROJECT_SOURCE_DIR}/docs/debugging.md"
    "${PROJECT_SOURCE_DIR}/docs/android-building.md"
    "${PROJECT_SOURCE_DIR}/docs/android-usage.md"
    "${PROJECT_SOURCE_DIR}/docs/android-backend.md"
    "${PROJECT_SOURCE_DIR}/docs/android-status.md"
)
install(FILES ${N120_PUBLIC_DOCS} DESTINATION docs)
foreach(document IN LISTS N120_PUBLIC_DOCS)
    get_filename_component(document_name "${document}" NAME)
    configure_file("${document}" "${CMAKE_BINARY_DIR}/docs/${document_name}" COPYONLY)
endforeach()
if(WIN32)
    get_target_property(qmake_path Qt6::qmake IMPORTED_LOCATION)
    get_filename_component(qt_bin_dir "${qmake_path}" DIRECTORY)
    find_program(N120_WINDEPLOYQT windeployqt HINTS "${qt_bin_dir}" REQUIRED)
    configure_file("${PROJECT_SOURCE_DIR}/cmake/DeployWindows.cmake.in" "${CMAKE_CURRENT_BINARY_DIR}/DeployWindows.cmake" @ONLY)
    install(SCRIPT "${CMAKE_CURRENT_BINARY_DIR}/DeployWindows.cmake")
endif()
add_custom_command(TARGET n120 POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_if_different "${PROJECT_SOURCE_DIR}/README.md" "$<TARGET_FILE_DIR:n120>/README.md"
    )

