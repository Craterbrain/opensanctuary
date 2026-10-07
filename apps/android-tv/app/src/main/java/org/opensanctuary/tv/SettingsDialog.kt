package org.opensanctuary.tv

import android.app.Dialog
import android.content.Context
import android.os.Bundle
import android.view.LayoutInflater
import android.view.Window
import android.widget.Button
import android.widget.EditText
import android.widget.RadioButton
import android.widget.TextView
import android.widget.Toast
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class SettingsDialog(
    context: Context,
    private val pairingManager: PairingManager,
    private val onSave: (serverIp: String, port: Int, isStageMode: Boolean) -> Unit,
    private val onResetPairing: () -> Unit,
) : Dialog(context) {

    private val prefs = context.getSharedPreferences("open_sanctuary_tv", Context.MODE_PRIVATE)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        requestWindowFeature(Window.FEATURE_NO_TITLE)

        val view = LayoutInflater.from(context).inflate(R.layout.dialog_settings, null)
        setContentView(view)

        val editIp = view.findViewById<EditText>(R.id.editServerIp)
        val editPort = view.findViewById<EditText>(R.id.editServerPort)
        val rbFoh = view.findViewById<RadioButton>(R.id.rbFohLive)
        val rbStage = view.findViewById<RadioButton>(R.id.rbStageView)
        val btnSave = view.findViewById<Button>(R.id.btnSave)
        val btnCancel = view.findViewById<Button>(R.id.btnCancel)
        val btnScan = view.findViewById<Button>(R.id.btnScanLan)
        val txtPairingStatus = view.findViewById<TextView>(R.id.txtPairingStatus)
        val btnResetPairing = view.findViewById<Button>(R.id.btnResetPairing)

        val savedIp = prefs.getString("server_ip", "") ?: ""
        val savedPort = prefs.getInt("server_port", 8443)
        val isStage = prefs.getBoolean("is_stage_mode", false)

        editIp.setText(savedIp)
        editPort.setText(savedPort.toString())
        if (isStage) {
            rbStage.isChecked = true
        } else {
            rbFoh.isChecked = true
        }

        fun refreshPairingStatus() {
            val pairedId = pairingManager.getStoredInstanceId()
            val hasToken = pairingManager.getStoredDeviceToken() != null
            txtPairingStatus.text = if (!hasToken || pairedId.isNullOrEmpty()) {
                context.getString(R.string.not_paired)
            } else {
                "Paired to console: $pairedId"
            }
        }
        refreshPairingStatus()

        // Clears the stored device_token/instance_id (PairingManager.clearPairing)
        // and re-runs MainActivity's connect logic (onResetPairing), which --
        // finding no stored token anymore -- falls straight into the manual
        // QR-and-poll pairing screen. Deliberate and explicit only, per
        // docs/CLIENT_PAIRING.md's "Resetting a pairing": never automatic.
        btnResetPairing.setOnClickListener {
            pairingManager.clearPairing()
            refreshPairingStatus()
            Toast.makeText(context, "Pairing reset. Scan the QR shown on this TV to pair it again.", Toast.LENGTH_LONG).show()
            dismiss()
            onResetPairing()
        }

        btnScan.setOnClickListener {
            btnScan.text = "Scanning..."
            btnScan.isEnabled = false
            CoroutineScope(Dispatchers.Main).launch {
                val port = editPort.text.toString().toIntOrNull() ?: 8443
                val found = NetworkDiscovery.scanLocalSubnet(context, port)
                btnScan.text = context.getString(R.string.scan_network)
                btnScan.isEnabled = true
                if (found.isNotEmpty()) {
                    editIp.setText(found.first())
                    Toast.makeText(context, "Found server: ${found.first()}", Toast.LENGTH_SHORT).show()
                } else {
                    Toast.makeText(context, "No OpenSanctuary servers found on local network", Toast.LENGTH_LONG).show()
                }
            }
        }

        btnCancel.setOnClickListener {
            dismiss()
        }

        btnSave.setOnClickListener {
            val ip = editIp.text.toString().trim()
            val port = editPort.text.toString().toIntOrNull() ?: 8443
            val stageMode = rbStage.isChecked

            if (ip.isEmpty()) {
                Toast.makeText(context, "Please enter a valid IP address", Toast.LENGTH_SHORT).show()
                return@setOnClickListener
            }

            // A pin established for the old address has no bearing on
            // whatever's at the new one -- see
            // PairingManager.clearPinnedCertFingerprint's doc comment.
            if (ip != savedIp || port != savedPort) {
                pairingManager.clearPinnedCertFingerprint()
            }

            prefs.edit()
                .putString("server_ip", ip)
                .putInt("server_port", port)
                .putBoolean("is_stage_mode", stageMode)
                .apply()

            onSave(ip, port, stageMode)
            dismiss()
        }
    }
}
