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
    property real rms: 0
    property real meter: 0
    property bool connected: false
    property bool holdVisible: false
    property var theme: ({})
    property int fontBase: 12

    readonly property bool showHud: holdVisible
        || (daemonState !== "idle" && daemonState !== "")
        || errorText.length > 0
        || daemonState === "offline"

    readonly property bool grabKeys: daemonState === "listening"
        || daemonState === "transcribing"
        || daemonState === "thinking"
        || daemonState === "executing"
        || daemonState === "speaking"

    // Omarchy bar uses the fontconfig "monospace" alias so `omarchy font set` applies.
    readonly property string hudFont: "monospace"
    readonly property int fontName: Math.max(1, Math.round(fontBase * 13 / 12))
    readonly property int fontReply: Math.max(1, Math.round(fontBase * 14 / 12))

    readonly property color cBg: theme.background || "#1e1e2e"
    readonly property color cCard: theme.lighter_background || "#313244"
    readonly property color cFg: theme.foreground || "#cdd6f4"
    readonly property color cMuted: theme.dark_foreground || "#6c7086"
    readonly property color cAccent: theme.accent || "#89b4fa"
    readonly property color cRed: theme.red || "#f38ba8"
    readonly property color cYellow: theme.yellow || "#f9e2af"
    readonly property color cGreen: theme.green || "#a6e3a1"

    readonly property color stateColor: daemonState === "listening" ? cRed
        : daemonState === "transcribing" ? cYellow
        : daemonState === "thinking" || daemonState === "executing" ? cAccent
        : daemonState === "speaking" ? cGreen
        : daemonState === "error" || daemonState === "offline" ? cRed
        : cMuted

    function parseTheme(text) {
        const out = {};
        const lines = (text || "").split("\n");
        for (let i = 0; i < lines.length; i++) {
            const m = lines[i].match(/^\s*([a-z_]+)\s*=\s*"([^"]+)"/);
            if (m)
                out[m[1]] = m[2];
        }
        return out;
    }

    function parseFontBase(text) {
        const lines = (text || "").split("\n");
        let inFont = false;
        let n = 0;
        for (let i = 0; i < lines.length; i++) {
            const line = lines[i].replace(/#.*$/, "").trim();
            const sec = line.match(/^\[([^\]]+)\]$/);
            if (sec) {
                inFont = sec[1] === "font";
                continue;
            }
            if (!inFont)
                continue;
            const m = line.match(/^base-size\s*=\s*([0-9]+(?:\.[0-9]+)?)/);
            if (m)
                n = Math.round(Number(m[1]));
        }
        return n >= 1 ? n : 0;
    }

    function meterFrac(rms) {
        const open = 0.004;
        const close = 0.0025;
        if (typeof rms !== "number" || !(rms > 0))
            return 0;
        if (root.meter <= 0 && rms < open)
            return 0;
        if (root.meter > 0 && rms < close)
            return 0;
        const db = 20 * Math.log10(rms);
        const gdb = 20 * Math.log10(open);
        return Math.max(0, Math.min(1, (db - gdb) / (-12 - gdb)));
    }

    function applyShellFonts() {
        let user = 0;
        let themeSz = 0;
        try {
            user = parseFontBase(userShell.text());
        } catch (e) {}
        try {
            themeSz = parseFontBase(themeShell.text());
        } catch (e) {}
        root.fontBase = user || themeSz || 12;
    }

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
            const wasOffline = root.daemonState === "offline";
            if (obj.state)
                root.daemonState = obj.state;
            if (wasOffline)
                hideTimer.restart();
            break;
        case "state":
            if (obj.state)
                root.daemonState = obj.state;
            if (obj.state === "listening") {
                root.transcript = "";
                root.assistant = "";
                root.errorText = "";
                root.rms = 0;
                root.meter = 0;
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
        case "level":
            if (typeof obj.rms === "number") {
                root.rms = obj.rms;
                root.meter = root.meterFrac(obj.rms);
            }
            break;
        case "error":
            root.errorText = obj.message || "error";
            root.daemonState = "error";
            root.holdVisible = true;
            hideTimer.restart();
            break;
        }
    }

    FileView {
        id: themeFile
        path: (Quickshell.env("HOME") || "") + "/.local/state/omarchy/current/theme/colors.toml"
        watchChanges: true
        printErrors: false
        onLoaded: root.theme = root.parseTheme(text())
        onLoadFailed: root.theme = ({})
        onFileChanged: reload()
    }

    FileView {
        id: themeShell
        path: (Quickshell.env("HOME") || "") + "/.local/state/omarchy/current/theme/shell.toml"
        watchChanges: true
        printErrors: false
        onLoaded: root.applyShellFonts()
        onLoadFailed: root.applyShellFonts()
        onFileChanged: reload()
    }

    FileView {
        id: userShell
        path: (Quickshell.env("HOME") || "") + "/.config/omarchy/shell.toml"
        watchChanges: true
        printErrors: false
        onLoaded: root.applyShellFonts()
        onLoadFailed: root.applyShellFonts()
        onFileChanged: reload()
    }

    Timer {
        interval: 2000
        running: true
        repeat: true
        onTriggered: {
            themeFile.reload()
            themeShell.reload()
            userShell.reload()
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
        id: cancelProc
        command: [(Quickshell.env("HOME") || "") + "/.local/bin/omarchy-assistant", "cancel"]
        running: false
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
                root.daemonState = "offline";
                root.transcript = "";
                root.assistant = "";
                root.errorText = "";
                root.holdVisible = true;
                hideTimer.stop();
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
        WlrLayershell.keyboardFocus: root.grabKeys ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.None

        Shortcut {
            sequences: ["Escape"]
            enabled: root.grabKeys
            onActivated: cancelProc.running = true
        }
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
            color: Qt.rgba(cBg.r, cBg.g, cBg.b, 0.90)
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
                        color: root.cFg
                        font.family: root.hudFont
                        font.pixelSize: root.fontName
                        font.bold: true
                    }
                }

                Rectangle {
                    visible: root.daemonState === "listening"
                    width: body.width
                    height: 6
                    radius: 3
                    color: root.cCard
                    Rectangle {
                        width: parent.width * root.meter
                        height: parent.height
                        radius: 3
                        color: root.stateColor
                    }
                }

                Text {
                    visible: root.transcript.length > 0
                    width: body.width
                    text: root.transcript
                    color: root.cMuted
                    font.family: root.hudFont
                    font.pixelSize: root.fontName
                    wrapMode: Text.Wrap
                }

                Text {
                    visible: root.assistant.length > 0
                    width: body.width
                    text: root.assistant
                    color: root.cFg
                    font.family: root.hudFont
                    font.pixelSize: root.fontReply
                    wrapMode: Text.Wrap
                }

                Text {
                    visible: root.daemonState === "offline"
                    width: body.width
                    text: "Waiting for omarchy-assistantd."
                    color: root.cMuted
                    font.family: root.hudFont
                    font.pixelSize: root.fontName
                    wrapMode: Text.Wrap
                }

                Text {
                    visible: root.errorText.length > 0
                    width: body.width
                    text: root.errorText
                    color: root.cRed
                    font.family: root.hudFont
                    font.pixelSize: root.fontName
                    wrapMode: Text.Wrap
                }
            }
        }
    }
}
