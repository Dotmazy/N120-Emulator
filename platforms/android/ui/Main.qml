import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import QtQuick.Dialogs
import N120.Display

ApplicationWindow {
    id: root
    width: 430
    height: 900
    visible: true
    title: "N120 Studio"
    Material.accent: Material.Cyan
    Material.primary: "#122334"
    Material.theme: emulator.setting("theme", 0) === 1 ? Material.Light : emulator.setting("theme", 0) === 2 ? Material.Dark : Material.System
    property string importKind: "internal"
    property string exportKind: "diagnostics"
    property bool visualKeys: emulator.setting("visualKeys", true)
    property bool dimLcd: emulator.setting("dimLcd", true)
    property var lcdIds: [0x4E4101, 0x4E4102, 0x4E4105, 0x4E4106, 0x4E4601, 0x4E4801, 0x4E5401]
    function openExport(kind) {
        root.exportKind = kind;
        exportDialog.selectedFile = emulator.exportSuggestion(kind);
        exportDialog.open();
    }
    color: Material.background
    onClosing: function (event) {
        emulator.releaseTouches();
        if (navigation.currentIndex !== 0) {
            navigation.currentIndex = 0;
            event.accepted = false;
        } else
            emulator.disconnectHost();
    }
    header: ToolBar {
        Material.theme: Material.Dark
        ColumnLayout {
            anchors.fill: parent
            anchors.leftMargin: 18
            anchors.rightMargin: 18
            spacing: 2
            RowLayout {
                Layout.fillWidth: true
                Label {
                    text: "N120"
                    font.pixelSize: 23
                    font.bold: true
                }
                Label {
                    text: "STUDIO"
                    font.pixelSize: 12
                    opacity: 0.6
                    Layout.fillWidth: true
                }
                Label {
                    text: "REMOTE"
                    font.pixelSize: 10
                    font.bold: true
                    color: Material.accent
                }
                Rectangle {
                    width: 7
                    height: 7
                    radius: 4
                    color: emulator.connected ? "#42d6a4" : "#a4aeb8"
                }
            }
            Label {
                text: emulator.status
                font.pixelSize: 11
                elide: Text.ElideRight
                Layout.fillWidth: true
                bottomPadding: 7
            }
        }
    }
    footer: ColumnLayout {
        spacing: 0
        Rectangle {
            visible: emulator.error.length > 0
            Layout.fillWidth: true
            implicitHeight: errorLabel.implicitHeight + 18
            color: Material.theme === Material.Dark ? "#532a27" : "#ffe5df"
            Label {
                id: errorLabel
                anchors.fill: parent
                anchors.margins: 9
                text: emulator.error
                wrapMode: Text.Wrap
                font.pixelSize: 12
            }
            TapHandler {
                onTapped: emulator.clearError()
            }
        }
        TabBar {
            id: navigation
            Layout.fillWidth: true
            TabButton {
                text: "Calculator"
            }
            TabButton {
                text: "Controls"
            }
            TabButton {
                text: "Developer"
            }
            TabButton {
                text: "Settings"
            }
            onCurrentIndexChanged: emulator.releaseTouches()
        }
    }
    FileDialog {
        id: importDialog
        title: "Import " + root.importKind
        nameFilters: ["Firmware images (*.bin)", "All files (*)"]
        onAccepted: emulator.importFile(selectedFile, root.importKind)
    }
    FileDialog {
        id: exportDialog
        fileMode: FileDialog.SaveFile
        title: "Export " + root.exportKind
        nameFilters: root.exportKind === "screenshot" ? ["PNG image (*.png)"] : root.exportKind === "flash" ? ["Flash image (*.bin)"] : ["Diagnostic report (*.txt)"]
        onAccepted: emulator.exportFile(selectedFile, root.exportKind)
    }
    Connections {
        target: emulator
        function onFlashReady() {
            root.openExport("flash");
        }
    }
    StackLayout {
        anchors.fill: parent
        currentIndex: navigation.currentIndex
        Item {
            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 8
                spacing: 3
                RowLayout {
                    Layout.fillWidth: true
                    Label {
                        text: emulator.running ? (emulator.paused ? "Paused" : "Running") : "Ready to connect"
                        font.pixelSize: 12
                        opacity: 0.7
                        Layout.fillWidth: true
                    }
                    Label {
                        text: emulator.running ? emulator.actualSpeed.toFixed(2) + "× · " + (Number(emulator.hardware.virtualMs || 0) / 1000).toFixed(1) + " s" : "Firmware runs on Windows"
                        font.pixelSize: 11
                        opacity: 0.7
                    }
                }
                Item {
                    id: calculatorArea
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    FocusScope {
                        id: calculator
                        objectName: "calculatorFocus"
                        focus: navigation.currentIndex === 0
                        width: Math.min(calculatorArea.width, calculatorArea.height * 1160 / 2221)
                        height: width * 2221 / 1160
                        anchors.centerIn: parent
                        Image {
                            anchors.fill: parent
                            source: "qrc:/assets/calculator.png"
                            fillMode: Image.PreserveAspectFit
                            asynchronous: true
                        }
                        LcdItem {
                            x: parent.width * 185 / 1160
                            y: parent.height * 184 / 2221
                            width: parent.width * 792 / 1160
                            height: parent.height * 594 / 2221
                            controller: emulator
                            Rectangle {
                                anchors.fill: parent
                                color: "black"
                                opacity: root.dimLcd ? (1 - Math.max(0, Math.min(1, emulator.backlight))) * 0.8 : 0
                            }
                        }
                        Rectangle {
                            x: parent.width * 535 / 1160
                            y: parent.height * 14 / 2221
                            width: parent.width * 90 / 1160
                            height: parent.height * 29 / 2221
                            radius: height / 2
                            color: emulator.led
                            border.color: "#8894a1"
                            border.width: 1
                        }
                        Repeater {
                            model: emulator.keys
                            delegate: Rectangle {
                                required property var modelData
                                x: calculator.width * modelData.x / 1160
                                y: calculator.height * modelData.y / 2221
                                width: calculator.width * modelData.w / 1160
                                height: calculator.height * modelData.h / 2221
                                radius: Math.min(width, height) / 2
                                property bool down: root.visualKeys && emulator.held.indexOf(modelData.index) >= 0
                                color: down ? "#6634b9df" : "transparent"
                                border.color: "#87d9ff"
                                border.width: down ? 1 : 0
                                Accessible.role: Accessible.Button
                                Accessible.name: modelData.name
                                Accessible.onPressAction: emulator.tapKey(modelData.index)
                            }
                        }
                        MultiPointTouchArea {
                            anchors.fill: parent
                            minimumTouchPoints: 1
                            maximumTouchPoints: 10
                            mouseEnabled: true
                            function updatePoints(points, down) {
                                calculator.forceActiveFocus();
                                for (let point of points)
                                    emulator.touch(point.pointId, point.x / width * 1160, point.y / height * 2221, down);
                            }
                            onPressed: function (points) {
                                updatePoints(points, true);
                            }
                            onUpdated: function (points) {
                                updatePoints(points, true);
                            }
                            onReleased: function (points) {
                                updatePoints(points, false);
                            }
                            onCanceled: emulator.releaseTouches()
                        }
                    }
                }
                RowLayout {
                    Layout.alignment: Qt.AlignHCenter
                    spacing: 12
                    Button {
                        text: emulator.running ? (emulator.paused ? "Resume" : "Pause") : "Start"
                        enabled: emulator.connected
                        onClicked: emulator.action(emulator.running ? (emulator.paused ? "resume" : "pause") : "start")
                    }
                    Button {
                        text: "Reset"
                        flat: true
                        enabled: emulator.running
                        onClicked: emulator.action("reset")
                    }
                }
            }
        }
        ScrollView {
            clip: true
            contentWidth: availableWidth
            ColumnLayout {
                width: parent.width
                spacing: 12
                Label {
                    text: "Simulation"
                    font.pixelSize: 24
                    font.bold: true
                    Layout.margins: 16
                }
                Label {
                    text: "Controls affect the firmware executing on the paired Windows host."
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    opacity: 0.7
                }
                Flow {
                    Layout.fillWidth: true
                    Layout.leftMargin: 12
                    Layout.rightMargin: 12
                    spacing: 6
                    Repeater {
                        model: [
                            {
                                t: "Start",
                                a: "start"
                            },
                            {
                                t: "Stop",
                                a: "stop"
                            },
                            {
                                t: "Pause",
                                a: "pause"
                            },
                            {
                                t: "Resume",
                                a: "resume"
                            },
                            {
                                t: "Reset MCU",
                                a: "reset"
                            },
                            {
                                t: "Reload firmware",
                                a: "reload"
                            },
                            {
                                t: "Restart engine",
                                a: "restart"
                            }
                        ]
                        delegate: Button {
                            required property var modelData
                            text: modelData.t
                            enabled: emulator.connected
                            onClicked: emulator.action(modelData.a)
                        }
                    }
                }
                Label {
                    text: "Virtual battery & power"
                    font.pixelSize: 18
                    font.bold: true
                    Layout.leftMargin: 16
                }
                Label {
                    text: batterySlider.value.toFixed(0) + "% · " + Number(emulator.hardware.batteryVoltage || 0).toFixed(2) + " V"
                    Layout.leftMargin: 16
                }
                Slider {
                    id: batterySlider
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    from: 0
                    to: 100
                    stepSize: 1
                    value: emulator.setting("battery", 100)
                    onMoved: emulator.battery(Math.round(value))
                }
                ComboBox {
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    model: ["Unplugged", "Charger", "Computer (USB on Windows host)"]
                    currentIndex: emulator.setting("cable", 0)
                    onActivated: emulator.cable(currentIndex)
                }
                Label {
                    text: "This battery is separate from the phone battery. Android does not expose an arbitrary USB gadget."
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    font.pixelSize: 12
                    opacity: 0.65
                }
                Label {
                    text: "Firmware & external flash"
                    font.pixelSize: 18
                    font.bold: true
                    Layout.leftMargin: 16
                }
                Label {
                    text: emulator.files
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    font.pixelSize: 12
                }
                Repeater {
                    model: [
                        {
                            kind: "internal",
                            title: "Internal · 512 KiB"
                        },
                        {
                            kind: "slotA",
                            title: "Slot A / full flash"
                        },
                        {
                            kind: "slotB",
                            title: "Slot B · 4 MiB"
                        },
                        {
                            kind: "flash",
                            title: "Restore flash backup"
                        }
                    ]
                    delegate: RowLayout {
                        required property var modelData
                        Layout.fillWidth: true
                        Layout.leftMargin: 16
                        Layout.rightMargin: 16
                        Label {
                            text: modelData.title
                            Layout.fillWidth: true
                            wrapMode: Text.Wrap
                            font.pixelSize: 13
                        }
                        Button {
                            text: "Import"
                            flat: true
                            onClicked: {
                                root.importKind = modelData.kind;
                                importDialog.open();
                            }
                        }
                        Button {
                            text: "Upload"
                            enabled: emulator.connected && !emulator.running
                            onClicked: emulator.upload(modelData.kind)
                        }
                    }
                }
                Label {
                    text: "Import copies to private phone storage. Upload sends to the host while stopped. External uploads replace their destination slot; back up first."
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    font.pixelSize: 12
                    opacity: 0.65
                }
                Button {
                    text: "Back up external flash"
                    Layout.leftMargin: 16
                    enabled: emulator.connected
                    onClicked: emulator.exportFlash()
                }
                Button {
                    text: "Save LCD screenshot"
                    Layout.leftMargin: 16
                    enabled: emulator.running
                    onClicked: {
                        root.openExport("screenshot");
                    }
                }
                Item {
                    height: 12
                }
            }
        }
        ScrollView {
            clip: true
            contentWidth: availableWidth
            ColumnLayout {
                width: parent.width
                spacing: 12
                Label {
                    text: "Developer"
                    font.pixelSize: 24
                    font.bold: true
                    Layout.margins: 16
                }
                Label {
                    text: "REMOTE / Renode\nPC  0x" + Number(emulator.hardware.pc || 0).toString(16).toUpperCase() + "\nSP  0x" + Number(emulator.hardware.sp || 0).toString(16).toUpperCase() + "\nInstructions  " + Number(emulator.hardware.instructions || 0).toLocaleString() + "\nVirtual time  " + (Number(emulator.hardware.virtualMs || 0) / 1000).toFixed(3) + " s\nMeasured host speed  " + emulator.actualSpeed.toFixed(3) + "×\nLCD initialized  " + Boolean(emulator.hardware.lcdActive) + "\nGamma valid  " + Boolean(emulator.hardware.gamma) + "\nUSB enumeration  Windows host only"
                    font.family: "monospace"
                    font.pixelSize: 13
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                }
                Button {
                    text: "Export diagnostics"
                    Layout.leftMargin: 16
                    onClicked: {
                        root.openExport("diagnostics");
                    }
                }
                Label {
                    text: "GDB and full register inspection remain available locally on Windows. The phone does not expose a remote monitor or GDB port."
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    font.pixelSize: 12
                    opacity: 0.65
                }
                Label {
                    text: "Hardware snapshot"
                    font.bold: true
                    Layout.leftMargin: 16
                }
                TextArea {
                    text: emulator.diagnostics
                    readOnly: true
                    selectByMouse: true
                    wrapMode: TextEdit.Wrap
                    font.family: "monospace"
                    font.pixelSize: 11
                    Layout.fillWidth: true
                    Layout.leftMargin: 12
                    Layout.rightMargin: 12
                }
                Label {
                    text: "Recent backend logs"
                    font.bold: true
                    Layout.leftMargin: 16
                }
                TextArea {
                    text: emulator.logs
                    readOnly: true
                    wrapMode: TextEdit.Wrap
                    font.family: "monospace"
                    font.pixelSize: 11
                    Layout.fillWidth: true
                    Layout.leftMargin: 12
                    Layout.rightMargin: 12
                }
            }
        }
        ScrollView {
            clip: true
            contentWidth: availableWidth
            ColumnLayout {
                width: parent.width
                spacing: 10
                Label {
                    text: "Settings"
                    font.pixelSize: 24
                    font.bold: true
                    Layout.margins: 16
                }
                Label {
                    text: "Emulation backend"
                    font.pixelSize: 18
                    font.bold: true
                    Layout.leftMargin: 16
                }
                Label {
                    text: "Authenticated Windows host. Standalone Android firmware execution is not implemented."
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    opacity: 0.7
                }
                TextField {
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    placeholderText: "Host IP (AVD: 10.0.2.2)"
                    text: emulator.setting("host", "")
                    inputMethodHints: Qt.ImhUrlCharactersOnly
                    onTextEdited: emulator.setSetting("host", text)
                }
                TextField {
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    placeholderText: "Port"
                    text: emulator.setting("port", 7420)
                    inputMethodHints: Qt.ImhDigitsOnly
                    validator: IntValidator {
                        bottom: 1
                        top: 65535
                    }
                    onTextEdited: emulator.setSetting("port", Number(text))
                }
                TextField {
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    placeholderText: "Host certificate SHA-256 fingerprint"
                    text: emulator.setting("pin", "")
                    selectByMouse: true
                    onTextEdited: emulator.setSetting("pin", text)
                }
                TextField {
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    placeholderText: "Pairing token"
                    text: emulator.setting("token", "")
                    echoMode: TextInput.Password
                    onTextEdited: emulator.setSetting("token", text)
                }
                RowLayout {
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    Button {
                        text: "Connect"
                        enabled: !emulator.connected
                        onClicked: emulator.connectHost()
                    }
                    Button {
                        text: "Disconnect"
                        enabled: emulator.connected
                        onClicked: emulator.disconnectHost()
                    }
                }
                Label {
                    text: "The fingerprint and token come from your private host setup. Traffic is encrypted. Use a trusted LAN; this is a single-controller service."
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    font.pixelSize: 12
                    opacity: 0.65
                }
                Label {
                    text: "Appearance & input"
                    font.pixelSize: 18
                    font.bold: true
                    Layout.leftMargin: 16
                }
                ComboBox {
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    model: ["System theme", "Light theme", "Dark theme"]
                    currentIndex: emulator.setting("theme", 0)
                    onActivated: {
                        emulator.setSetting("theme", currentIndex);
                        root.Material.theme = currentIndex === 1 ? Material.Light : currentIndex === 2 ? Material.Dark : Material.System;
                    }
                }
                Switch {
                    text: "Visual key feedback"
                    checked: root.visualKeys
                    Layout.leftMargin: 16
                    onToggled: {
                        root.visualKeys = checked;
                        emulator.setSetting("visualKeys", checked);
                    }
                }
                Switch {
                    text: "Visualize LCD backlight"
                    checked: root.dimLcd
                    Layout.leftMargin: 16
                    onToggled: {
                        root.dimLcd = checked;
                        emulator.setSetting("dimLcd", checked);
                    }
                }
                Label {
                    text: "Physical keyboards: arrows, F1 OK, F2 Power, F3 Shift, F4 Alpha, Enter EXE. Type letters for physical key sequences. Android Back navigates the app."
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    font.pixelSize: 12
                    opacity: 0.65
                }
                Label {
                    text: "Emulation & display"
                    font.pixelSize: 18
                    font.bold: true
                    Layout.leftMargin: 16
                }
                Switch {
                    text: "Pause when app enters background"
                    checked: emulator.setting("pauseBackground", true)
                    Layout.leftMargin: 16
                    onToggled: emulator.setSetting("pauseBackground", checked)
                }
                Label {
                    text: "Speed target (host pacing)"
                    Layout.leftMargin: 16
                }
                ComboBox {
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    model: ["Unlimited", "0.25×", "0.5×", "1×", "2×"]
                    property var values: [0, 0.25, 0.5, 1, 2]
                    currentIndex: Math.max(0, values.indexOf(emulator.setting("speed", 1)))
                    onActivated: emulator.speedTarget(values[currentIndex])
                }
                Label {
                    text: "LCD identity · applied at next Start"
                    Layout.leftMargin: 16
                }
                ComboBox {
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    model: root.lcdIds.map(id => "0x" + id.toString(16).toUpperCase())
                    currentIndex: Math.max(0, root.lcdIds.indexOf(emulator.setting("lcd", 0x4E4101)))
                    enabled: !emulator.running
                    onActivated: emulator.setSetting("lcd", root.lcdIds[currentIndex])
                }
                Label {
                    text: "320 × 240 RGB565 · nearest-neighbor pixels\nPhone storage is private. Flash persists on the Windows host with periodic atomic checkpoints. Export backups to keep a phone copy."
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                    Layout.leftMargin: 16
                    Layout.rightMargin: 16
                    font.pixelSize: 12
                    opacity: 0.65
                }
                Label {
                    text: "Not available: local execution, phone USB gadget, local GDB, haptic feedback and soft keyboard translation. No inactive controls are shown."
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                    Layout.margins: 16
                    font.pixelSize: 12
                    opacity: 0.65
                }
            }
        }
    }
}
