' Server IP/port/mode settings dialog, backed by an roRegistrySection so the values
' persist across app restarts (Roku's equivalent of Android's SharedPreferences).

sub init()
    m.ipText = m.top.findNode("ipText")
    m.portText = m.top.findNode("portText")
    m.modeButtonGroup = m.top.findNode("modeButtonGroup")
    m.actionButtonGroup = m.top.findNode("actionButtonGroup")

    m.actionButtonGroup.observeField("buttonSelected", "onActionSelected")
    m.modeButtonGroup.observeField("buttonSelected", "onModeSelected")

    sec = CreateObject("roRegistrySection", "OpenSanctuary")
    if sec.Exists("server_ip") then m.top.serverIp = sec.Read("server_ip")
    if sec.Exists("server_port") then m.top.serverPort = sec.Read("server_port")
    if sec.Exists("display_mode") then m.top.displayMode = sec.Read("display_mode")

    updateUI()
end sub

sub updateUI()
    m.ipText.text = m.top.serverIp
    m.portText.text = m.top.serverPort
end sub

sub openDialog()
    m.top.visible = true
    updateUI()
    m.actionButtonGroup.setFocus(true)
end sub

sub onModeSelected()
    if m.modeButtonGroup.buttonSelected = 1
        m.top.displayMode = "foldback"
    else
        m.top.displayMode = "live"
    end if
end sub

sub onActionSelected()
    idx = m.actionButtonGroup.buttonSelected
    if idx = 0
        promptKeyboardIp()
    else if idx = 1
        savePreferences()
        m.top.visible = false
        m.top.closed = true
    else if idx = 2
        m.top.visible = false
        m.top.closed = true
    end if
end sub

sub promptKeyboardIp()
    kb = CreateObject("roSGNode", "StandardKeyboardDialog")
    kb.title = "Enter OpenSanctuary Server IP Address"
    kb.text = m.top.serverIp
    kb.buttons = ["OK", "Cancel"]
    kb.observeField("buttonSelected", "onIpKeyboardDone")
    m.top.getScene().dialog = kb
end sub

sub onIpKeyboardDone(event as Object)
    dialog = event.getRoSGNode()
    if dialog.buttonSelected = 0
        m.top.serverIp = dialog.text
        updateUI()
    end if
    dialog.close = true
end sub

sub savePreferences()
    sec = CreateObject("roRegistrySection", "OpenSanctuary")
    sec.Write("server_ip", m.top.serverIp)
    sec.Write("server_port", m.top.serverPort)
    sec.Write("display_mode", m.top.displayMode)
    sec.Flush()
end sub

function onKeyEvent(key as String, press as Boolean) as Boolean
    if press and key = "back"
        m.top.visible = false
        m.top.closed = true
        return true
    end if
    return false
end function
