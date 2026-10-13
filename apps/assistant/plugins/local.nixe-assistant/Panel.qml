import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

Panel {
  id: root
  moduleName: "local.nixe-assistant"
  ipcTarget: "local.nixe-assistant"

  property var anchorItem: null
  property var hostWidget: null
  property var status: ({ ok: false, state: "offline", service_active: false, muted: false, name: "Assistant" })
  property string statusMessage: ""

  readonly property color fg: root.bar ? root.bar.foreground : Color.foreground
  readonly property color dim: Qt.darker(fg, 1.45)
  readonly property color warn: "#e8ba87"
  readonly property color green: "#a6e3a1"
  readonly property color red: "#f38ba8"
  readonly property color accent: "#89b4fa"
  readonly property string fontFamily: root.bar ? root.bar.fontFamily : Style.font.family

  readonly property string helper: Qt.resolvedUrl("bin/assistant-bar-action").toString().replace(/^file:\/\//, "")

  readonly property bool isServiceActive: root.status.service_active === true
  readonly property bool isOnline: root.status.ok === true
  readonly property bool isMuted: root.status.muted === true
  readonly property string daemonState: root.status.state || (root.isServiceActive ? "starting" : "disabled")

  // Bar icon exposed to BarWidget.qml
  readonly property string barIcon: !root.isServiceActive ? "󰍭"
    : !root.isOnline ? "󰍭"
    : root.isMuted ? "󰍭"
    : root.daemonState === "speaking" ? "󰔊"
    : root.daemonState === "listening" ? "󰍬"
    : "󰍬"

  readonly property color barIconColor: !root.isServiceActive ? root.dim
    : !root.isOnline ? root.dim
    : root.isMuted ? root.warn
    : root.daemonState === "speaking" ? root.green
    : root.daemonState === "listening" ? root.accent
    : (root.bar ? root.bar.barForeground : Color.foreground)

  readonly property string assistantName: (root.status && root.status.name) ? root.status.name : "Assistant"

  readonly property string tooltip: !root.isServiceActive ? root.assistantName + " · Disabled (Models Unloaded)"
    : !root.isOnline ? root.assistantName + " · Starting Up..."
    : root.isMuted ? root.assistantName + " · Muted (Capture Paused)"
    : root.assistantName + ((root.status.restart_required || []).length ? " · Restart required" : "") + " · " + (root.daemonState.toUpperCase()) + (root.status.session ? " · " + root.status.session.mode : "") + (root.status.model ? " · " + root.status.model.split("/").pop() : "")

  function open() { root.controller.show(); root.refresh() }
  function close() { root.controller.hide() }
  function toggle() { root.opened ? close() : open() }
  function switchPanel(direction) {
    if (root.bar && typeof root.bar.switchPanelFrom === "function") return root.bar.switchPanelFrom(root.hostWidget || root, direction)
    return false
  }

  function refresh() {
    statusProc.running = false
    statusProc.running = true
  }

  function toggleService() {
    run(["toggle-service"], root.isServiceActive ? "Stopping assistant & unloading models..." : "Starting assistant & loading models...")
  }
  function toggleMute() { run(["toggle-mute"], "Toggling mute...") }
  function testVoice() { run(["demo"], "Playing test greeting...") }
  function reloadDaemon() { run(["reload"], "Reloading config...") }
  function openTui() { run(["open-tui"], "Opening Settings TUI...") }
  function restartService() { run(["restart"], "Restarting assistant services...") }

  function run(args, msg) {
    if (msg) root.statusMessage = msg
    actionProc.command = [root.helper].concat(args)
    actionProc.running = false
    actionProc.running = true
  }

  Timer {
    interval: 1800
    running: true
    repeat: true
    onTriggered: root.refresh()
  }

  Process {
    id: statusProc
    command: [root.helper, "status"]
    running: true
    stdout: SplitParser {
      splitMarker: "\n"
      onRead: function(line) {
        var str = (line || "").trim()
        if (!str.length) return
        try {
          var parsed = JSON.parse(str)
          root.status = parsed
          if (root.statusMessage.length > 0 && !actionProc.running) {
            root.statusMessage = ""
          }
        } catch (e) {}
      }
    }
  }

  Process {
    id: actionProc
    running: false
    onExited: function(exitCode, exitStatus) {
      if (exitCode !== 0) root.statusMessage = "Action failed (exit " + exitCode + ") — see Diagnostics"
    }
    onRunningChanged: {
      if (!running) {
        Qt.callLater(root.refresh)
      }
    }
  }

  KeyboardPanel {
    id: panel
    anchorItem: root.anchorItem
    owner: root.hostWidget || root
    bar: root.bar
    open: root.opened
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(430))
    contentHeight: panel.fittedContentHeight(Math.min(content.implicitHeight, Style.space(620)))

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }
      onTextKey: function(text) {
        if (text === "d") root.toggleService()
        else if (text === "m") root.toggleMute()
        else if (text === "t") root.testVoice()
        else if (text === "r") root.reloadDaemon()
        else if (text === "s") root.openTui()
      }

      Column {
        id: content
        width: parent.width
        spacing: Style.space(12)

        // Header
        Item {
          width: parent.width
          implicitHeight: Math.max(titleBlock.implicitHeight, actions.implicitHeight)

          Row {
            id: titleBlock
            anchors.left: parent.left
            anchors.right: actions.left
            anchors.rightMargin: Style.space(8)
            spacing: Style.space(10)

            Text {
              text: root.barIcon
              color: root.barIconColor
              font.family: root.fontFamily
              font.pixelSize: Style.font.title * 1.5
              anchors.verticalCenter: parent.verticalCenter
            }

            Column {
              anchors.verticalCenter: parent.verticalCenter
              spacing: Style.space(2)

              Text {
                text: root.assistantName
                color: root.fg
                font.family: root.fontFamily
                font.pixelSize: Style.font.title
                font.bold: true
              }

              Text {
                text: !root.isServiceActive ? "Disabled · Models Unloaded"
                  : !root.isOnline ? "Starting up..."
                  : root.isMuted ? "Audio Capture Muted"
                  : "Active (" + root.daemonState.toUpperCase() + ")"
                color: root.barIconColor
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
              }
            }
          }

          Row {
            id: actions
            anchors.right: parent.right
            anchors.top: parent.top
            spacing: Style.space(4)

            PanelActionButton {
              iconText: root.isServiceActive ? "󰓛" : "󰐥"
              tooltipText: root.isServiceActive ? "Disable Assistant (d)" : "Enable Assistant (d)"
              foreground: root.isServiceActive ? root.red : root.green
              onClicked: root.toggleService()
            }
            PanelActionButton {
              iconText: root.isMuted ? "󰍬" : "󰍭"
              tooltipText: root.isMuted ? "Unmute Mic (m)" : "Mute Mic (m)"
              foreground: root.isMuted ? root.warn : root.fg
              enabled: root.isServiceActive
              onClicked: root.toggleMute()
            }
            PanelActionButton {
              iconText: "󰔊"
              tooltipText: "Voice Preview (t)"
              foreground: root.fg
              enabled: root.isServiceActive
              onClicked: root.testVoice()
            }
            PanelActionButton {
              iconText: "󰑐"
              tooltipText: "Refresh / Reload (r)"
              foreground: root.fg
              onClicked: root.refresh()
            }
          }
        }

        // Assistant Master Memory & Power Switch
        Rectangle {
          width: parent.width
          implicitHeight: powerCol.implicitHeight + Style.space(16)
          radius: Style.cornerRadius
          color: root.isServiceActive ? Qt.rgba(root.green.r, root.green.g, root.green.b, 0.08) : Qt.rgba(root.warn.r, root.warn.g, root.warn.b, 0.08)
          border.width: 1
          border.color: root.isServiceActive ? Qt.rgba(root.green.r, root.green.g, root.green.b, 0.25) : Qt.rgba(root.warn.r, root.warn.g, root.warn.b, 0.25)

          Row {
            id: powerCol
            anchors.fill: parent
            anchors.margins: Style.space(10)
            spacing: Style.space(10)

            Column {
              width: parent.width - toggleBtn.width - Style.space(10)
              anchors.verticalCenter: parent.verticalCenter
              spacing: Style.space(2)

              Row {
                spacing: Style.space(6)
                Text {
                  text: root.isServiceActive ? "●" : "○"
                  color: root.isServiceActive ? root.green : root.warn
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                }
                Text {
                  text: root.isServiceActive ? "Assistant Running (Models in VRAM)" : "Assistant Disabled (Memory Freed)"
                  color: root.fg
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                  font.bold: true
                }
              }

              Text {
                text: root.isServiceActive
                  ? "Stop daemon to release its model memory."
                  : "Assistant daemon is stopped."
                color: root.dim
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption * 0.9
                wrapMode: Text.WordWrap
                width: parent.width
              }
            }

            Button {
              id: toggleBtn
              anchors.verticalCenter: parent.verticalCenter
              text: root.isServiceActive ? "Disable" : "Enable"
              iconText: root.isServiceActive ? "󰓛" : "󰐥"
              fontSize: Style.font.caption
              foreground: root.isServiceActive ? root.red : root.green
              fontFamily: root.fontFamily
              bordered: true
              onClicked: root.toggleService()
            }
          }
        }

        PanelSeparator { foreground: root.fg }

        // Telemetry details card
        Rectangle {
          width: parent.width
          implicitHeight: infoCol.implicitHeight + Style.space(16)
          radius: Style.cornerRadius
          color: Qt.rgba(root.fg.r, root.fg.g, root.fg.b, 0.06)
          border.width: 1
          border.color: Qt.rgba(root.fg.r, root.fg.g, root.fg.b, 0.12)

          Column {
            id: infoCol
            anchors.fill: parent
            anchors.margins: Style.space(10)
            spacing: Style.space(6)

            Row {
              spacing: Style.space(6)
              Text {
                text: "Active LLM:"
                color: root.dim
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
                width: Style.space(100)
              }
              Text {
                text: !root.isServiceActive ? "None (unloaded from VRAM)" : (root.status.model ? root.status.model.split("/").pop() : "Loading...")
                color: root.isServiceActive ? root.fg : root.dim
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
                font.bold: root.isServiceActive
                elide: Text.ElideMiddle
                width: content.width - Style.space(130)
              }
            }

            Row {
              spacing: Style.space(6)
              Text {
                text: "Kokoro Voice:"
                color: root.dim
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
                width: Style.space(100)
              }
              Text {
                text: (root.status.tts_voice || "af_heart") + " (" + (root.status.tts_variant || "fp32") + ")"
                color: root.fg
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
              }
            }

            Row {
              spacing: Style.space(6)
              Text {
                text: "Hotkey:"
                color: root.dim
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
                width: Style.space(100)
              }
              Text {
                text: root.isServiceActive ? "Alt + Space (Push-to-Talk)" : "Inactive while disabled"
                color: root.isServiceActive ? root.accent : root.dim
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
              }
            }
          }
        }

        // Action Buttons Row 1
        Row {
          width: parent.width
          spacing: Style.space(6)

          Button {
            width: (content.width - Style.space(6)) / 2
            text: root.isMuted ? "Unmute Mic" : "Mute Mic"
            iconText: root.isMuted ? "󰍬" : "󰍭"
            fontSize: Style.font.caption
            foreground: root.fg
            fontFamily: root.fontFamily
            bordered: true
            enabled: root.isServiceActive
            onClicked: root.toggleMute()
          }

          Button {
            width: (content.width - Style.space(6)) / 2
            text: "Test Voice"
            iconText: "󰔊"
            fontSize: Style.font.caption
            foreground: root.fg
            fontFamily: root.fontFamily
            bordered: true
            enabled: root.isServiceActive
            onClicked: root.testVoice()
          }
        }

        // Action Buttons Row 2
        Row {
          width: parent.width
          spacing: Style.space(6)

          Button {
            width: (content.width - Style.space(6)) / 2
            text: "Reload Config"
            iconText: "󰑓"
            fontSize: Style.font.caption
            foreground: root.fg
            fontFamily: root.fontFamily
            bordered: true
            enabled: root.isServiceActive
            onClicked: root.reloadDaemon()
          }

          Button {
            width: (content.width - Style.space(6)) / 2
            text: "Restart Services"
            iconText: "󰜎"
            fontSize: Style.font.caption
            foreground: root.fg
            fontFamily: root.fontFamily
            bordered: true
            onClicked: root.restartService()
          }
        }

        Row {
          width: parent.width; spacing: Style.space(6)
          Button { width: (content.width - Style.space(6)) / 2; text: root.status.session && root.status.session.mode === "conversation" ? "End Conversation" : "Conversation"; fontSize: Style.font.caption; fontFamily: root.fontFamily; bordered: true; enabled: root.isOnline; onClicked: root.run(["conversation",root.status.session && root.status.session.mode === "conversation" ? "stop" : "start"],"Changing conversation mode...") }
          Button { width: (content.width - Style.space(6)) / 2; text: "Resume Browser"; fontSize: Style.font.caption; fontFamily: root.fontFamily; bordered: true; enabled: root.isOnline; onClicked: root.run(["browser-resume"],"Resuming browser...") }
        }

        // Prominent Button to launch Settings TUI
        Button {
          width: parent.width
          text: "Open Settings & Model Manager TUI"
          iconText: "󰘳"
          fontSize: Style.font.caption
          foreground: root.accent
          fontFamily: root.fontFamily
          bordered: true
          selected: true
          onClicked: {
            root.openTui()
            root.close()
          }
        }

        Text {
          visible: root.statusMessage !== ""
          width: parent.width
          horizontalAlignment: Text.AlignHCenter
          text: root.statusMessage
          color: root.accent
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
        }
      }
    }
  }
}
