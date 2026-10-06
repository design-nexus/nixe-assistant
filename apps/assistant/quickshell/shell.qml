// Nyx HUD. Standalone Quickshell overlay — do not add to shell.json yet.
//
//   qs --path /home/ken/Projects/omarchy-assistant/quickshell --daemonize --no-duplicate
//
// Listens to `omarchy-assistant listen` NDJSON on stdout.

import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland

ShellRoot {
    id: root

    property string daemonState: "idle"
    property string transcript: ""
    property string assistant: ""
    property string errorText: ""
    property string name: "Nyx"
    property bool connected: false
    property bool holdVisible: false

    readonly property bool showHud: holdVisible
        || (daemonState !== "idle" && daemonState !== "")
        || errorText.length > 0

    readonly property color stateColor: daemonState === "listening" ? "#e06c75"
        : daemonState === "transcribing" ? "#e5c07b"
        : daemonState === "thinking" || daemonState === "executing" ? "#61afef"
        : daemonState === "speaking" ? "#98c379"
        : daemonState === "error" ? "#e06c75"
        : "#abb2bf"

    function handleLine(line) {
        const trimmed = (line || "").trim();
        if (!trimmed.length)
            return;
        let obj;
        try {
            obj = JSON.parse(trimmed);
        } catch (e) {
            return;
        }
        root.connected = true;
        switch (obj.type) {
        case "health":
            if (obj.name)
                root.name = obj.name;
            if (obj.state)
                root.daemonState = obj.state;
            break;
        case "state":
            if (obj.state)
                root.daemonState = obj.state;
            if (obj.state === "listening") {
                root.transcript = "";
                root.assistant = "";
                root.errorText = "";
                root.holdVisible = true;
                hideTimer.stop();
            } else if (obj.state === "idle") {
                hideTimer.restart();
            } else {
                root.holdVisible = true;
                hideTimer.stop();
            }
            break;
        case "partial_transcript":
        case "transcript":
            if (typeof obj.text === "string")
                root.transcript = obj.text;
            break;
        case "assistant_text":
            if (typeof obj.text === "string")
                root.assistant = obj.text;
            break;
        case "error":
            root.errorText = obj.message || "error";
            root.daemonState = "error";
            root.holdVisible = true;
            hideTimer.restart();
            break;
        }
    }

    Timer {
        id: hideTimer
        interval: 2500
        repeat: false
        onTriggered: {
            root.holdVisible = false;
            if (root.daemonState === "error")
                root.daemonState = "idle";
            root.errorText = "";
        }
    }

    Process {
        id: listener
        command: [(Quickshell.env("HOME") || "") + "/.local/bin/omarchy-assistant", "listen"]
        running: true

        stdout: SplitParser {
            splitMarker: "\n"
            onRead: function (data) {
                root.handleLine(data);
            }
        }

        onRunningChanged: {
            if (!listener.running) {
                root.connected = false;
                restartTimer.restart();
            }
        }
    }

    Timer {
        id: restartTimer
        interval: 1000
        repeat: false
        onTriggered: {
            if (!listener.running)
                listener.running = true;
        }
    }

    PanelWindow {
        id: panel
        visible: true
        color: "transparent"
        anchors {
            top: true
            bottom: true
            left: true
            right: true
        }

        WlrLayershell.namespace: "nyx-hud"
        WlrLayershell.layer: WlrLayer.Overlay
        WlrLayershell.keyboardFocus: WlrKeyboardFocus.None
        exclusionMode: ExclusionMode.Ignore

        mask: Region {
            intersection: Intersection.Subtract
            x: 0
            y: 0
            width: panel.width
            height: panel.height
        }

        Rectangle {
            id: card
            visible: root.showHud
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.top: parent.top
            anchors.topMargin: 20
            width: 440
            implicitHeight: body.implicitHeight + 28
            radius: 12
            color: Qt.rgba(0.10, 0.10, 0.12, 0.90)
            border.width: 1
            border.color: root.stateColor

            Column {
                id: body
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: 14
                spacing: 6

                Row {
                    spacing: 8
                    Rectangle {
                        width: 8
                        height: 8
                        radius: 4
                        color: root.stateColor
                        anchors.verticalCenter: parent.verticalCenter
                    }
                    Text {
                        text: root.name + "  ·  " + root.daemonState
                        color: "#dcdfe4"
                        font.pixelSize: 13
                        font.bold: true
                    }
                }

                Text {
                    visible: root.transcript.length > 0
                    width: body.width
                    text: root.transcript
                    color: "#abb2bf"
                    font.pixelSize: 13
                    wrapMode: Text.Wrap
                }

                Text {
                    visible: root.assistant.length > 0
                    width: body.width
                    text: root.assistant
                    color: "#f8f8f2"
                    font.pixelSize: 14
                    wrapMode: Text.Wrap
                }

                Text {
                    visible: root.errorText.length > 0
                    width: body.width
                    text: root.errorText
                    color: "#e06c75"
                    font.pixelSize: 13
                    wrapMode: Text.Wrap
                }
            }
        }
    }
}
