import QtQuick
import qs.Commons
import qs.Ui

BarWidget {
  id: root
  moduleName: "local.omarchy-assistant"

  function injectPanel() {
    if (!panelLoader.item) return
    panelLoader.item.bar = root.bar
    panelLoader.item.anchorItem = button
    panelLoader.item.hostWidget = root
  }

  readonly property bool opened: panelLoader.item ? panelLoader.item.opened === true : false
  function open() { if (panelLoader.item) panelLoader.item.open() }
  function close() { if (panelLoader.item) panelLoader.item.close() }
  function closeForPopoutSwitch() { if (panelLoader.item) panelLoader.item.close() }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight
  onBarChanged: injectPanel()

  Loader {
    id: panelLoader
    active: true
    visible: false
    source: Qt.resolvedUrl("Panel.qml")
    onLoaded: { root.injectPanel(); Qt.callLater(root.injectPanel) }
    onStatusChanged: if (status === Loader.Error) console.warn("local.omarchy-assistant: Panel.qml failed to load")
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: panelLoader.item ? panelLoader.item.barIcon : "󰍬"
    foreground: panelLoader.item ? panelLoader.item.barIconColor : (root.bar ? root.bar.barForeground : Color.foreground)
    slotSize: Style.bar.statusSlot
    fontSize: Style.font.caption
    tooltipText: panelLoader.item && panelLoader.item.tooltip ? panelLoader.item.tooltip : ((panelLoader.item && panelLoader.item.status && panelLoader.item.status.name) ? panelLoader.item.status.name : "Assistant")
    active: root.opened

    onPressed: function(buttonCode) {
      if (buttonCode === Qt.RightButton) {
        if (panelLoader.item && panelLoader.item.isServiceActive) {
          panelLoader.item.toggleMute()
        } else {
          root.opened ? root.close() : root.open()
        }
      } else if (buttonCode === Qt.MiddleButton) {
        if (panelLoader.item && panelLoader.item.isServiceActive) {
          panelLoader.item.testVoice()
        } else {
          root.opened ? root.close() : root.open()
        }
      } else {
        root.opened ? root.close() : root.open()
      }
    }
  }
}
