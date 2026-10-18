// Nixe Assistant HUD overlay
//
// Installed to ~/.local/share/nixe-assistant/quickshell
//
// Listens to `nixe-assistant listen` NDJSON on stdout.

import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland

ShellRoot {
    id: root

    property string daemonState: "idle"
    property string sessionMode: "wake"
    property int sessionRemaining: 0
    property string dictationDraft: ""
    property string dictationTarget: ""
    property string toolProgress: ""
    property string transcript: ""
    property string assistant: ""
    property string errorText: ""
    property string name: "Assistant"
    property real rms: 0
    property real meter: 0
    property bool hudEnabled: true
    property string approvalId: ""
    property string approvalText: ""
    property double approvalExpires: 0
    property bool approvalTaskScope: false
    property string approvalDetail: ""
    property bool approvalVoice: false
    property bool approvalDetailsOpen: false
    // "Type a note in Write · step 2 of 4", while a task runs.
    property string taskLine: ""
    property string taskPlan: ""
    property bool connected: false
    property bool holdVisible: false
    property var theme: ({})
    property int fontBase: 12

    readonly property bool showHud: hudEnabled && connected && daemonState !== "offline"
        && (approvalId.length > 0 || holdVisible
            || (daemonState !== "idle" && daemonState !== "")
            || errorText.length > 0)

    // False while the assistant types or clicks in another app (its keys must reach that app).
    property bool computerInput: false
    readonly property bool grabKeys: hudEnabled && connected && !computerInput && (approvalId.length > 0 || (sessionMode === "wake" && daemonState === "listening")
        || (sessionMode === "wake" && daemonState === "transcribing")
        || daemonState === "thinking"
        || daemonState === "executing"
        || daemonState === "speaking")

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
        case "session":
            root.sessionMode = obj.mode; root.sessionRemaining = obj.remaining_secs;
            root.dictationDraft = obj.draft || ""; root.dictationTarget = obj.target || "";
            break;
        case "settings":
            root.hudEnabled = obj.hud_enabled !== false;
            break;
        case "computer_input":
            root.computerInput = obj.active === true;
            break;
        case "tool_approval":
            root.approvalId = obj.approval_id;
            root.approvalExpires = obj.expires_at_ms;
            root.approvalTaskScope = obj.task_scope_allowed === true;
            root.approvalVoice = obj.voice_allowed === true;
            root.approvalText = obj.summary || obj.tool;
            root.approvalDetail = obj.tool + "\n" + JSON.stringify(obj.arguments, null, 2);
            root.approvalDetailsOpen = false;
            break;
        case "turn":
            if (obj.event && obj.event.type === "tool") { root.toolProgress = obj.event.tool + " · " + obj.event.status + " · " + JSON.stringify(obj.event.arguments || {}); }
            if (obj.event && obj.event.type === "task" && obj.event.task) {
                const t = obj.event.task;
                const running = ["queued", "planning", "running", "waiting_for_approval", "paused"].indexOf(t.state) >= 0;
                const request = t.request.length > 60 ? t.request.substring(0, 60) + "…" : t.request;
                root.taskLine = running ? request + " · step " + Math.min(t.calls + 1, t.max_calls) + " of " + t.max_calls + (t.state === "waiting_for_approval" ? " · waiting for you" : "") : "";
                root.taskPlan = running && t.plan && t.plan.length > 0 ? t.plan.map((s, i) => (i + 1) + ". " + s).join("\n") : "";
            }
            if (obj.event && obj.event.type === "calibration") {
                root.errorText = "Calibration: " + obj.event.phase + " for five seconds";
                hideTimer.restart();
            }
            break;
        case "result":
            if (obj.data && obj.data.approved !== undefined) { root.approvalId = ""; root.approvalText = ""; }
            break;
        case "health":
            if (obj.name)
                root.name = obj.name;
            if (obj.state)
                root.daemonState = obj.state;
            break;
        case "state":
            if (obj.state)
                root.daemonState = obj.state;
            if (obj.state === "idle" || obj.state === "interrupted" || obj.state === "error") {
                root.approvalId = ""; root.approvalText = "";
            }
            if (obj.state === "listening") {
                root.taskLine = "";
                root.taskPlan = "";
                root.toolProgress = "";
                root.transcript = "";
                root.assistant = "";
                root.errorText = "";
                root.rms = 0;
                root.meter = 0;
                root.holdVisible = false;
                root.approvalId = "";
                root.approvalText = "";
                root.dictationDraft = "";
                root.toolProgress = "";
                hideTimer.stop();
            } else if (obj.state === "idle") {
                hideTimer.restart();
            } else {
                root.holdVisible = false;
                root.approvalId = "";
                root.approvalText = "";
                root.dictationDraft = "";
                root.toolProgress = "";
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

    Timer {
        interval: 500; running: root.approvalId.length > 0; repeat: true
        onTriggered: if (Date.now() >= root.approvalExpires) {root.approvalId = ""; root.approvalText = "";}
    }
    Process { id: approvalProc }
    // Answer the pending approval: "approve" or "deny", once or for the rest of the task.
    function decide(verb, forTask) {
        approvalProc.command = ["nixe-assistant", verb, root.approvalId].concat(forTask ? ["--task"] : []);
        approvalProc.running = true;
        root.approvalId = "";
        root.approvalText = "";
    }
    Process {
        id: cancelProc
        command: [(Quickshell.env("HOME") || "") + "/.local/bin/nixe-assistant", "cancel"]
        running: false
    }

    Process {
        id: listener
        command: [(Quickshell.env("HOME") || "") + "/.local/bin/nixe-assistant", "listen"]
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
                root.holdVisible = false;
                root.approvalId = "";
                root.approvalText = "";
                root.dictationDraft = "";
                root.toolProgress = "";
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
        visible: root.hudEnabled && root.connected
        color: "transparent"
        anchors {
            top: true
            bottom: true
            left: true
            right: true
        }

        WlrLayershell.namespace: "nixe-assistant-hud"
        WlrLayershell.layer: WlrLayer.Overlay
        WlrLayershell.keyboardFocus: root.grabKeys ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.None

        Shortcut {
            sequence: "Y"; enabled: root.approvalId.length > 0 && root.hudEnabled && root.connected
            onActivated: root.decide("approve", false)
        }
        Shortcut {
            sequence: "T"; enabled: root.approvalId.length > 0 && root.approvalTaskScope && root.hudEnabled && root.connected
            onActivated: root.decide("approve", true)
        }
        Shortcut {
            sequence: "N"; enabled: root.approvalId.length > 0 && root.hudEnabled && root.connected
            onActivated: root.decide("deny", false)
        }
        Shortcut {
            sequences: ["Escape"]
            enabled: root.grabKeys
            onActivated: cancelProc.running = true
        }
        exclusionMode: ExclusionMode.Ignore

        // Clicks pass through, except on the card while it asks for an approval.
        mask: Region { item: root.approvalId.length > 0 ? card : noInput }
        Item { id: noInput; width: 0; height: 0 }

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
                        text: root.name + "  ·  " + root.daemonState + (root.sessionMode === "wake" ? "" : " · " + root.sessionMode + " " + root.sessionRemaining + "s")
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

                Text { visible: root.toolProgress.length > 0 && root.daemonState !== "idle"; width: body.width; text: root.toolProgress; color: root.cMuted; font.family: root.hudFont; wrapMode: Text.WrapAnywhere }
                Text { visible: root.dictationDraft.length > 0; width: body.width; text: "Draft for " + root.dictationTarget + ": " + root.dictationDraft; color: root.cFg; font.family: root.hudFont; wrapMode: Text.WrapAnywhere }
                Text { visible: root.taskLine.length > 0 && root.daemonState !== "idle"; width: body.width; text: root.taskLine; color: root.cMuted; font.family: root.hudFont; wrapMode: Text.Wrap }
                Text { visible: root.taskPlan.length > 0 && root.daemonState !== "idle"; width: body.width; text: root.taskPlan; color: root.cMuted; font.family: root.hudFont; wrapMode: Text.Wrap }
                Column {
                    visible: root.approvalId.length > 0
                    width: body.width
                    spacing: 8
                    Text { width: parent.width; text: root.approvalText; color: root.cFg; font.family: root.hudFont; font.pixelSize: root.fontReply; font.bold: true; wrapMode: Text.Wrap }
                    Text {
                        text: (root.approvalDetailsOpen ? "▾ " : "▸ ") + "Details"
                        color: root.cMuted; font.family: root.hudFont
                        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.approvalDetailsOpen = !root.approvalDetailsOpen }
                    }
                    Flickable {
                        visible: root.approvalDetailsOpen
                        width: parent.width; height: Math.min(160, detailLabel.implicitHeight)
                        contentHeight: detailLabel.implicitHeight; clip: true
                        Text { id: detailLabel; width: parent.width; text: root.approvalDetail; color: root.cMuted; font.family: root.hudFont; wrapMode: Text.WrapAnywhere }
                    }
                    Row {
                        spacing: 8
                        Repeater {
                            model: [
                                {label: "Allow  Y", verb: "approve", task: false, shown: true, main: true},
                                {label: "Allow for task  T", verb: "approve", task: true, shown: root.approvalTaskScope, main: false},
                                {label: "Deny  N", verb: "deny", task: false, shown: true, main: false}
                            ]
                            Rectangle {
                                required property var modelData
                                visible: modelData.shown
                                width: buttonLabel.implicitWidth + 24; height: buttonLabel.implicitHeight + 12
                                radius: 6
                                color: modelData.main ? root.cAccent : (buttonMouse.containsMouse ? root.cCard : "transparent")
                                border.width: 1; border.color: modelData.main ? root.cAccent : root.cMuted
                                Text { id: buttonLabel; anchors.centerIn: parent; text: parent.modelData.label; color: parent.modelData.main ? root.cBg : root.cFg; font.family: root.hudFont; font.bold: parent.modelData.main }
                                MouseArea { id: buttonMouse; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: root.decide(parent.modelData.verb, parent.modelData.task) }
                            }
                        }
                    }
                    Text {
                        width: parent.width
                        text: root.approvalVoice ? "Or say “approve” or “deny”." : "Click or press a key to allow this; saying it doesn't count."
                        color: root.cMuted; font.family: root.hudFont; wrapMode: Text.Wrap
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
