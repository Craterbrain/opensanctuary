' Background worker: polls GET /api/display-state on m.top.pollIntervalMs and exposes
' the parsed body via m.top.displayState for MainScene to observeField on.

sub init()
    m.top.functionName = "runPollLoop"
end sub

sub runPollLoop()
    port = CreateObject("roMessagePort")
    urlTransfer = CreateObject("roUrlTransfer")
    urlTransfer.SetMessagePort(port)
    urlTransfer.SetCertificatesFile("common:/certs/ca-bundle.crt")
    urlTransfer.InitClientCertificates()

    lastUrl = ""

    while true
        currentUrl = m.top.serverUrl
        if currentUrl <> ""
            if currentUrl <> lastUrl
                lastUrl = currentUrl
                urlTransfer.SetUrl(currentUrl)
            end if

            urlTransfer.AsyncGetToString()
            msg = wait(1500, port)

            if type(msg) = "roUrlEvent"
                code = msg.GetResponseCode()
                if code = 200
                    parsed = ParseJson(msg.GetString())
                    if parsed <> invalid
                        m.top.connectionStatus = "connected"
                        m.top.displayState = parsed
                    else
                        m.top.connectionStatus = "error"
                    end if
                else
                    m.top.connectionStatus = "error"
                end if
            else
                urlTransfer.AsyncCancel()
                m.top.connectionStatus = "timeout"
            end if
        else
            m.top.connectionStatus = "no_url"
            sleep(1000)
        end if

        sleep(int(m.top.pollIntervalMs))
    end while
end sub
