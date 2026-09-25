' Front-of-house live output: renders the current slide's background + text, and
' drives the Video node's play/pause/seek when a dedicated media item (not just a
' slide's ambient background loop) is the live content.

sub init()
    m.bgColorRect = m.top.findNode("bgColorRect")
    m.bgPoster = m.top.findNode("bgPoster")
    m.bgVideo = m.top.findNode("bgVideo")
    m.lyricsLabel = m.top.findNode("lyricsLabel")
    m.footerTitle = m.top.findNode("footerTitle")
    m.footerAuthor = m.top.findNode("footerAuthor")
    m.alertBanner = m.top.findNode("alertBanner")
    m.alertText = m.top.findNode("alertText")
    m.blackoutOverlay = m.top.findNode("blackoutOverlay")
    m.logoOverlay = m.top.findNode("logoOverlay")

    m.currentVideoUrl = ""
    m.currentPosterUrl = ""
    m.serverBase = ""

    ' Drift tolerance before issuing a hard seek. The Video node has no runtime
    ' playback-rate control (no equivalent of HTML5's playbackRate), so unlike the web
    ' client's ~350ms rate-nudge threshold, small drift here is simply tolerated rather
    ' than corrected away — only a seek can correct it, and seeks risk a brief visible
    ' glitch on some devices/content.
    m.driftToleranceSec = 1.0
end sub

sub setServerBase(base as String)
    m.serverBase = base
end sub

function resolveUrl(path as String) as String
    if Left(path, 4) = "http" then return path
    fullPath = path
    if Left(fullPath, 1) <> "/" then fullPath = "/" + fullPath
    return m.serverBase + fullPath
end function

function cssHexToRokuColor(value as Dynamic) as String
    if value = invalid or type(value) <> "roString" then return "0x102027FF"
    if Left(value, 1) <> "#" then return "0x102027FF"
    hex = Mid(value, 2)
    if Len(hex) = 6 then return "0x" + hex + "FF"
    if Len(hex) = 8 then return "0x" + hex
    return "0x102027FF"
end function

' liveObj: the "live" object from /api/display-state, or invalid when nothing is live.
' mediaPlayback: the "media_playback" object, or invalid.
sub applyState(liveObj as Dynamic, mediaPlayback as Dynamic, isBlackout as Boolean, isClearText as Boolean, isLogo as Boolean, alertMessage as Dynamic)
    m.blackoutOverlay.visible = isBlackout
    m.logoOverlay.visible = (not isBlackout) and isLogo

    if alertMessage <> invalid and alertMessage <> ""
        m.alertText.text = Chr(55356) + Chr(56812) + " " + alertMessage
        m.alertBanner.visible = true
    else
        m.alertBanner.visible = false
    end if

    if isBlackout or isLogo then return

    if liveObj = invalid
        m.lyricsLabel.text = ""
        m.footerTitle.text = ""
        m.footerAuthor.text = ""
        renderBackground("solid", "#102027")
        return
    end if

    renderBackground(liveObj.background_kind, liveObj.background_value)

    if mediaPlayback <> invalid and liveObj.background_kind = "video"
        syncDedicatedMedia(mediaPlayback)
    end if

    slideText = ""
    if not isClearText and liveObj.current_slide_text <> invalid
        slideText = liveObj.current_slide_text
    end if
    m.lyricsLabel.text = slideText

    ' Auto-size by text length so short single lines get a large, readable treatment
    ' and long passages (e.g. full scripture verses) still fit.
    textLen = Len(slideText)
    if textLen <= 60
        m.lyricsLabel.font = "font:ExtraLargeBoldSystemFont"
    else if textLen <= 140
        m.lyricsLabel.font = "font:LargeBoldSystemFont"
    else if textLen <= 260
        m.lyricsLabel.font = "font:MediumBoldSystemFont"
    else
        m.lyricsLabel.font = "font:SmallBoldSystemFont"
    end if

    if isClearText
        m.footerTitle.text = ""
        m.footerAuthor.text = ""
    else
        m.footerTitle.text = liveObj.title
        if liveObj.subtitle <> invalid
            m.footerAuthor.text = liveObj.subtitle
        else
            m.footerAuthor.text = ""
        end if
    end if
end sub

sub renderBackground(kind as String, value as Dynamic)
    if kind = "video" and value <> invalid and value <> ""
        videoUrl = resolveUrl(value)
        if videoUrl <> m.currentVideoUrl
            m.currentVideoUrl = videoUrl
            content = CreateObject("roSGNode", "ContentNode")
            content.url = videoUrl
            content.streamFormat = "mp4"
            m.bgVideo.content = content
        end if
        m.bgVideo.visible = true
        m.bgPoster.visible = false
        m.bgColorRect.visible = false
    else if kind = "image" and value <> invalid and value <> ""
        imgUrl = resolveUrl(value)
        if imgUrl <> m.currentPosterUrl
            m.currentPosterUrl = imgUrl
            m.bgPoster.uri = imgUrl
        end if
        m.bgPoster.visible = true
        m.bgVideo.visible = false
        m.bgVideo.control = "stop"
        m.currentVideoUrl = ""
        m.bgColorRect.visible = false
    else
        m.bgColorRect.color = cssHexToRokuColor(value)
        m.bgColorRect.visible = true
        m.bgPoster.visible = false
        m.bgVideo.visible = false
        m.bgVideo.control = "stop"
        m.currentVideoUrl = ""
    end if
end sub

' Drives the background Video node from server-authoritative dedicated-media state —
' the same protocol every client (web, Android TV, Roku) now shares (see
' MediaPlaybackState in src/core/models.rs). `current_time`/`timestamp_ms` are a
' snapshot as of the server's last update, not a live position, so a playing item's
' target position is projected forward using elapsed wall-clock time first.
sub syncDedicatedMedia(mp as Object)
    isPlaying = (mp.is_playing = true)
    isLooping = (mp.is_looping = true)
    isMuted = (mp.is_muted = true)
    volume = mp.volume
    if volume = invalid then volume = 1.0

    m.bgVideo.loop = isLooping
    m.bgVideo.mute = isMuted

    dt = CreateObject("roDateTime")
    nowMs = (dt.AsSeconds() * 1000.0) + dt.GetMilliseconds()

    targetTime = mp.current_time
    if isPlaying
        startAnchorMs = mp.timestamp_ms
        if mp.start_at_epoch_ms <> invalid
            startAnchorMs = mp.start_at_epoch_ms
        end if
        elapsedSec = (nowMs - startAnchorMs) / 1000.0
        if elapsedSec > 0 then targetTime = mp.current_time + elapsedSec
    end if

    if isPlaying
        if m.bgVideo.state = "paused" or m.bgVideo.state = "stopped" or m.bgVideo.state = "none" or m.bgVideo.state = "finished" or m.bgVideo.state = "stopping"
            m.bgVideo.seek = targetTime
            m.bgVideo.control = "play"
        else
            drift = Abs(m.bgVideo.position - targetTime)
            if drift > m.driftToleranceSec then m.bgVideo.seek = targetTime
        end if
    else
        m.bgVideo.control = "pause"
        drift = Abs(m.bgVideo.position - targetTime)
        if drift > m.driftToleranceSec then m.bgVideo.seek = targetTime
    end if
end sub
