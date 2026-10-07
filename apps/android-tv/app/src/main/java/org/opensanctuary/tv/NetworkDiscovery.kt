package org.opensanctuary.tv

import android.content.Context
import android.net.wifi.WifiManager
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.Socket

object NetworkDiscovery {

    suspend fun scanLocalSubnet(context: Context, port: Int = 8443): List<String> = withContext(Dispatchers.IO) {
        val foundIps = mutableListOf<String>()
        try {
            val wifiManager = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
            val ipAddress = wifiManager?.connectionInfo?.ipAddress ?: 0
            if (ipAddress != 0) {
                val ipString = String.format(
                    "%d.%d.%d.",
                    ipAddress and 0xff,
                    ipAddress shr 8 and 0xff,
                    ipAddress shr 16 and 0xff
                )

                // Scan common host range quickly with short timeouts
                for (i in 1..254) {
                    val targetIp = "$ipString$i"
                    try {
                        val socket = Socket()
                        socket.connect(InetSocketAddress(targetIp, port), 60)
                        socket.close()
                        foundIps.add(targetIp)
                    } catch (_: Exception) {
                        // Port closed / host unreachable
                    }
                }
            }
        } catch (e: Exception) {
            e.printStackTrace()
        }
        foundIps
    }
}
