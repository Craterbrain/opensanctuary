' Top-level controller: owns the poll loop's URL/lifecycle, routes each poll's parsed
' GET /api/display-state payload to whichever view (Live or Foldback) is active, and
' hosts the settings dialog. See docs/CLIENT_PAIRING.md for this console's identity
' story — pairing/hijack-prevention enforcement is a documented future addition, not
' implemented by this client yet, matching the Android TV app's current scope.

sub init()
    m.liveView = m.top.findNode("liveView")
    m.foldbackView = m.top.findNode("foldbackView")
    m.statusBanner = m.top.findNode("statusBanner")
    m.statusBannerText = m.top.findNode("statusBannerText")
    m.settingsScreen = m.top.findNode("settingsScreen")
    m.pollTask = m.top.findNode("pollTask")

    m.serverIp = "192.168.1.100"
    m.serverPort = "8080"
    m.displayMode = "live"

    sec = CreateObject("roRegistrySection", "OpenSanctuary")
    if sec.Exists("server_ip") then m.serverIp = sec.Read("server_ip")
    if sec.Exists("server_port") then m.serverPort = sec.Read("server_port")
    if sec.Exists("display_mode") then m.displayMode = sec.Read("display_mode")

    m.pollTask.observeField("displayState", "onDisplayStateChanged")
    m.pollTask.observeField("connectionStatus", "onConnectionStatusChanged")
    m.settingsScreen.observeField("closed", "onSettingsClosed")

    applyDisplayMode()
    restartPolling()
end sub

sub restartPolling()
    base = "http://" + m.serverIp + ":" + m.serverPort
    m.liveView.callFunc("setServerBase", base)
    m.pollTask.serverUrl = base + "/api/display-state"
    m.pollTask.control = "RUN"
end sub

sub applyDisplayMode()
    isFoldback = (m.displayMode = "foldback")
    m.liveView.visible = not isFoldback
    m.foldbackView.visible = isFoldback
end sub

sub onConnectionStatusChanged()
    status = m.pollTask.connectionStatus
    if status = "connected"
        m.statusBanner.visible = false
    else
        m.statusBanner.visible = true
        m.statusBannerText.text = "Searching for OpenSanctuary at " + m.serverIp + ":" + m.serverPort + "..."
    end if
end sub

sub onSettingsClosed()
    sec = CreateObject("roRegistrySection", "OpenSanctuary")
    if sec.Exists("server_ip") then m.serverIp = sec.Read("server_ip")
    if sec.Exists("server_port") then m.serverPort = sec.Read("server_port")
    if sec.Exists("display_mode") then m.displayMode = sec.Read("display_mode")

    applyDisplayMode()
    restartPolling()
end sub

sub onDisplayStateChanged()
    state = m.pollTask.displayState
    if state = invalid then return

    isBlackout = (state.is_blackout = true)
    isClearText = (state.is_clear_text = true)
    isLogo = (state.is_logo_override = true)
    alertMessage = state.alert_message
    mediaPlayback = state.media_playback
    liveObj = state.live

    if m.displayMode = "foldback"
        m.foldbackView.callFunc("applyState", liveObj, mediaPlayback, isBlackout, isClearText, isLogo, alertMessage)
    else
        m.liveView.callFunc("applyState", liveObj, mediaPlayback, isBlackout, isClearText, isLogo, alertMessage)
    end if
end sub

function onKeyEvent(key as String, press as Boolean) as Boolean
    if press
        if key = "options" or key = "info"
            m.settingsScreen.openDialog()
            return true
        else if key = "play"
            restartPolling()
            return true
        end if
    end if
    return false
end function
