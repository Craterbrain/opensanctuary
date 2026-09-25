' Stage foldback / confidence monitor rendering.

sub init()
    m.statusBadge = m.top.findNode("statusBadge")
    m.currentTitle = m.top.findNode("currentTitle")
    m.currentText = m.top.findNode("currentText")
    m.notesText = m.top.findNode("notesText")
    m.nextTitle = m.top.findNode("nextTitle")
    m.nextText = m.top.findNode("nextText")
end sub

sub applyState(liveObj as Dynamic, mediaPlayback as Dynamic, isBlackout as Boolean, isClearText as Boolean, isLogo as Boolean, alertMessage as Dynamic)
    badge = ""
    if isBlackout then badge = "BLACKOUT"
    if badge = "" and isLogo then badge = "LOGO"
    if badge = "" and isClearText then badge = "TEXT CLEARED"
    m.statusBadge.text = badge

    if liveObj = invalid
        m.currentTitle.text = ""
        m.currentText.text = ""
        m.notesText.text = ""
        m.nextTitle.text = "Next Item / End of Presentation"
        m.nextText.text = ""
        return
    end if

    slideIndexLabel = ""
    if liveObj.slide_index <> invalid and liveObj.slide_count <> invalid
        slideIndexLabel = " (Slide " + (liveObj.slide_index + 1).ToStr() + " of " + liveObj.slide_count.ToStr() + ")"
    end if
    m.currentTitle.text = liveObj.title + slideIndexLabel
    m.currentText.text = liveObj.current_slide_text

    if liveObj.speaker_notes <> invalid and liveObj.speaker_notes <> ""
        m.notesText.text = liveObj.speaker_notes
    else
        m.notesText.text = "—"
    end if

    if liveObj.next_slide_text <> invalid and liveObj.next_slide_text <> ""
        m.nextTitle.text = "Next Slide"
        m.nextText.text = liveObj.next_slide_text
    else
        m.nextTitle.text = "Next Item / End of Presentation"
        m.nextText.text = "—"
    end if
end sub
