import jsQR from 'jsqr';
import { escapeHtml } from './core/presentation_helpers';

/**
 * Universal Mobile QR-Bridge Scanner for OpenSanctuary Client Pairing.
 * Supports iOS Safari, Android Chrome/Firefox/Samsung, and any browser with camera or file input.
 */

(function () {
  const urlParams = new URLSearchParams(window.location.search);
  let sessionToken = urlParams.get('key') || '';

  // DOM Elements
  const sessionBadge = document.getElementById('session-badge');
  const alertBanner = document.getElementById('alert-banner');
  const videoEl = document.getElementById('camera-feed') as HTMLVideoElement | null;
  const placeholderEl = document.getElementById('camera-placeholder');
  const reticleEl = document.getElementById('reticle-overlay');
  const startCameraBtn = document.getElementById('btn-start-camera');
  const snapPhotoBtn = document.getElementById('btn-snap-photo');
  const fileInput = document.getElementById('file-qr-input') as HTMLInputElement | null;
  const toggleManualBtn = document.getElementById('btn-toggle-manual');
  const manualCard = document.getElementById('manual-card');
  const scannerCard = document.getElementById('scanner-card');
  const successCard = document.getElementById('success-card');
  const manualForm = document.getElementById('manual-pairing-form') as HTMLFormElement | null;
  const successMsg = document.getElementById('success-message');
  const successDetails = document.getElementById('success-details');
  const pairAnotherBtn = document.getElementById('btn-pair-another');

  // Scanner State
  let videoStream: MediaStream | null = null;
  let isScanning = false;
  let lastScanTimestamp = 0;
  const SCAN_INTERVAL_MS = 80; // ~12 fps frame check to preserve mobile battery

  // Offscreen canvas for frame pixel processing
  const procCanvas = document.createElement('canvas');
  const procCtx = procCanvas.getContext('2d', { willReadFrequently: true });

  // Optional native BarcodeDetector if supported by host browser
  let nativeBarcodeDetector: any = null;
  if (typeof (window as any).BarcodeDetector !== 'undefined') {
    try {
      const getFormats = (window as any).BarcodeDetector.getSupportedFormats;
      if (typeof getFormats === 'function') {
        getFormats.call((window as any).BarcodeDetector).then((formats: any) => {
          if (formats && Array.isArray(formats) && formats.includes('qr_code')) {
            nativeBarcodeDetector = new (window as any).BarcodeDetector({ formats: ['qr_code'] });
          }
        }).catch(() => {});
      } else {
        nativeBarcodeDetector = new (window as any).BarcodeDetector({ formats: ['qr_code'] });
      }
    } catch (_) {}
  }

  function showAlert(msg: string, isError: boolean = true) {
    if (!alertBanner) return;
    alertBanner.textContent = msg;
    alertBanner.className = isError ? 'alert-banner alert-banner--error' : 'alert-banner alert-banner--success';
    alertBanner.style.display = 'block';
  }

  function hideAlert() {
    if (alertBanner) alertBanner.style.display = 'none';
  }

  // Initialize Session Badge
  function updateSessionBadge() {
    if (!sessionBadge) return;
    if (!sessionToken) {
      sessionBadge.textContent = '⚠ No session key attached';
      sessionBadge.style.color = '#ff5252';
      sessionBadge.style.borderColor = 'rgba(255, 82, 82, 0.4)';
      showAlert('Missing session key in URL. Scan the QR code from the console Remote modal first.');
    } else {
      sessionBadge.textContent = 'Session Key: ' + sessionToken.slice(0, 18) + '…';
      sessionBadge.style.color = '#00e5ff';
      sessionBadge.style.borderColor = 'rgba(0, 229, 255, 0.25)';
      hideAlert();
    }
  }
  updateSessionBadge();

  // Toggle Manual Input Card
  if (toggleManualBtn && manualCard) {
    toggleManualBtn.addEventListener('click', () => {
      if (manualCard.style.display === 'none' || !manualCard.style.display) {
        stopCamera();
        if (placeholderEl) placeholderEl.style.display = 'flex';
        manualCard.style.display = 'block';
        toggleManualBtn.textContent = '📷 Back to Camera Scanner';
      } else {
        manualCard.style.display = 'none';
        toggleManualBtn.textContent = '✍ Manual TV Code Entry';
        startCamera();
      }
    });
  }

  // Stop Camera
  function stopCamera() {
    isScanning = false;
    if (videoStream) {
      videoStream.getTracks().forEach(track => track.stop());
      videoStream = null;
    }
    if (videoEl) {
      videoEl.srcObject = null;
    }
    if (reticleEl) {
      reticleEl.style.display = 'none';
    }
  }

  // Start Camera with universal iOS/Android constraints
  async function startCamera() {
    hideAlert();
    if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
      showAlert('Live camera stream not supported by this browser. Use "Snap / Select Photo" or manual entry.');
      if (manualCard) manualCard.style.display = 'block';
      return;
    }

    if (!videoEl) return;

    // iOS Safari requires playsinline and muted properties & attributes
    videoEl.muted = true;
    videoEl.playsInline = true;
    videoEl.setAttribute('playsinline', 'true');
    videoEl.setAttribute('webkit-playsinline', 'true');
    videoEl.setAttribute('autoplay', 'true');
    videoEl.setAttribute('muted', 'true');

    try {
      // Ideal constraint prioritizing back/environment camera
      try {
        videoStream = await navigator.mediaDevices.getUserMedia({
          video: {
            facingMode: { ideal: 'environment' },
            width: { ideal: 1280 },
            height: { ideal: 720 },
          },
          audio: false,
        });
      } catch (errFirst) {
        console.warn('Initial camera constraint failed, falling back to generic video constraint:', errFirst);
        // Fallback for devices that fail on strict facingMode or resolution
        videoStream = await navigator.mediaDevices.getUserMedia({
          video: true,
          audio: false,
        });
      }

      videoEl.srcObject = videoStream;
      await videoEl.play().catch(e => console.warn('Video play error:', e));

      if (placeholderEl) placeholderEl.style.display = 'none';
      if (reticleEl) reticleEl.style.display = 'flex';

      isScanning = true;
      requestAnimationFrame(scanFrameLoop);
    } catch (err: any) {
      console.error('Camera access error:', err);
      showAlert('Camera permission denied or camera unavailable: ' + (err.message || err));
      if (manualCard) manualCard.style.display = 'block';
    }
  }

  if (startCameraBtn) {
    startCameraBtn.addEventListener('click', startCamera);
  }

  // Auto-start camera if session token is present
  if (sessionToken && navigator.mediaDevices && typeof navigator.mediaDevices.getUserMedia === 'function') {
    startCamera().catch(() => {});
  }

  // Continuous frame scanning loop
  async function scanFrameLoop(timestamp: number) {
    if (!isScanning || !videoEl) return;

    if (videoEl.readyState >= 2 && timestamp - lastScanTimestamp >= SCAN_INTERVAL_MS) {
      lastScanTimestamp = timestamp;

      const videoWidth = videoEl.videoWidth;
      const videoHeight = videoEl.videoHeight;

      if (videoWidth > 0 && videoHeight > 0) {
        // 1. Try native BarcodeDetector fast path if available
        let detectedValue: string | null = null;
        if (nativeBarcodeDetector) {
          try {
            const barcodes = await nativeBarcodeDetector.detect(videoEl);
            if (!isScanning) return;
            if (barcodes && barcodes.length > 0 && barcodes[0].rawValue) {
              detectedValue = barcodes[0].rawValue;
            }
          } catch (_) {}
        }

        // 2. Universal jsQR fallback running on canvas frame
        if (!detectedValue && procCtx) {
          // Downscale to max 800px dimension for rapid decode without thermal throttling
          let targetWidth = videoWidth;
          let targetHeight = videoHeight;
          const maxDim = 800;
          if (targetWidth > maxDim || targetHeight > maxDim) {
            if (targetWidth > targetHeight) {
              targetHeight = Math.round((targetHeight * maxDim) / targetWidth);
              targetWidth = maxDim;
            } else {
              targetWidth = Math.round((targetWidth * maxDim) / targetHeight);
              targetHeight = maxDim;
            }
          }

          procCanvas.width = targetWidth;
          procCanvas.height = targetHeight;
          procCtx.drawImage(videoEl, 0, 0, targetWidth, targetHeight);

          try {
            const imgData = procCtx.getImageData(0, 0, targetWidth, targetHeight);
            const qrResult = jsQR(imgData.data, targetWidth, targetHeight, {
              inversionAttempts: 'attemptBoth',
            });
            if (qrResult && qrResult.data) {
              detectedValue = qrResult.data;
            }
          } catch (e) {
            console.error('jsQR frame decoding error:', e);
          }
        }

        if (!isScanning) return;

        if (detectedValue) {
          handleScannedData(detectedValue);
          return; // Stop animation loop once handled
        }
      }
    }

    if (isScanning) {
      requestAnimationFrame(scanFrameLoop);
    }
  }

  // Snap / Select Photo via file input (Universal Fallback)
  if (snapPhotoBtn && fileInput) {
    snapPhotoBtn.addEventListener('click', () => {
      // Stop live camera to prevent camera hardware conflict on mobile devices
      stopCamera();
      if (placeholderEl) placeholderEl.style.display = 'flex';
      fileInput.click();
    });

    fileInput.addEventListener('change', async () => {
      const file = fileInput.files?.[0];
      // Reset input value so selecting the same file again triggers change event
      fileInput.value = '';
      if (!file) return;

      hideAlert();
      const img = new Image();
      const objUrl = URL.createObjectURL(file);

      img.onload = () => {
        URL.revokeObjectURL(objUrl);
        if (!procCtx) return;

        let w = img.naturalWidth || img.width;
        let h = img.naturalHeight || img.height;
        const maxDim = 1200;
        if (w > maxDim || h > maxDim) {
          if (w > h) {
            h = Math.round((h * maxDim) / w);
            w = maxDim;
          } else {
            w = Math.round((w * maxDim) / h);
            h = maxDim;
          }
        }

        procCanvas.width = w;
        procCanvas.height = h;
        procCtx.drawImage(img, 0, 0, w, h);

        try {
          const imgData = procCtx.getImageData(0, 0, w, h);
          const qrResult = jsQR(imgData.data, w, h, {
            inversionAttempts: 'attemptBoth',
          });

          if (qrResult && qrResult.data) {
            handleScannedData(qrResult.data);
          } else {
            showAlert('No QR code detected in the selected image. Please try again or enter code manually.');
          }
        } catch (err: any) {
          showAlert('Failed to process image: ' + err.message);
        }
      };

      img.onerror = () => {
        URL.revokeObjectURL(objUrl);
        showAlert('Could not read selected image file.');
      };

      img.src = objUrl;
    });
  }

  // Handle scanned raw string
  function handleScannedData(dataStr: string) {
    stopCamera();
    let payload: { device_id?: string; name?: string; platform?: string } = {};

    try {
      payload = JSON.parse(dataStr);
    } catch (_) {
      // Format: device_id:name:platform
      const parts = dataStr.split(':');
      if (parts.length >= 2) {
        payload = {
          device_id: parts[0].trim(),
          name: parts[1].trim(),
          platform: (parts[2] || 'android-tv').trim(),
        };
      } else {
        payload = {
          device_id: dataStr.trim(),
          name: 'Sanctuary Display',
          platform: 'android-tv',
        };
      }
    }

    submitPairingAuthorization(
      payload.device_id || 'unknown_tv',
      payload.name || 'Sanctuary Display',
      payload.platform || 'android-tv'
    );
  }

  // Manual Form Submission
  if (manualForm) {
    manualForm.addEventListener('submit', (e) => {
      e.preventDefault();
      const devIdEl = document.getElementById('input-device-id') as HTMLInputElement | null;
      const devNameEl = document.getElementById('input-device-name') as HTMLInputElement | null;
      const platformEl = document.getElementById('select-platform') as HTMLSelectElement | null;

      const deviceId = devIdEl?.value.trim() || '';
      const deviceName = devNameEl?.value.trim() || 'Sanctuary Display';
      const platform = platformEl?.value || 'android-tv';

      if (!deviceId) return;
      submitPairingAuthorization(deviceId, deviceName, platform);
    });
  }

  // Submit Authorization Request to Backend
  async function submitPairingAuthorization(deviceId: string, deviceName: string, platform: string) {
    hideAlert();
    const cleanId = (deviceId || '').trim();
    if (!cleanId) {
      showAlert('Device ID cannot be empty. Please scan a valid TV code or enter it manually.');
      return;
    }

    if (!sessionToken) {
      showAlert('Cannot authorize: No session token provided. Scan the QR code from the console first.');
      return;
    }

    const submitBtn = document.getElementById('btn-submit-manual') as HTMLButtonElement | null;
    if (submitBtn) {
      submitBtn.disabled = true;
      submitBtn.textContent = 'Authorizing…';
    }

    try {
      const res = await fetch('/api/pairing/authorize', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          session_token: sessionToken,
          device_id: deviceId,
          name: deviceName,
          platform: platform,
        }),
      });

      if (!res.ok) {
        const errJson = await res.json().catch(() => ({}));
        throw new Error(errJson.error || errJson.message || `HTTP ${res.status}`);
      }

      const data = await res.json();
      showSuccessScreen(data);
    } catch (err: any) {
      showAlert('Pairing Authorization Failed: ' + err.message);
      if (submitBtn) {
        submitBtn.disabled = false;
        submitBtn.textContent = 'Authorize Display';
      }
      if (videoStream === null && placeholderEl) {
        placeholderEl.style.display = 'flex';
      }
    }
  }

  // Show Success Card
  function showSuccessScreen(data: any) {
    stopCamera();
    if (scannerCard) scannerCard.style.display = 'none';
    if (manualCard) manualCard.style.display = 'none';
    if (successCard) successCard.style.display = 'flex';

    if (successMsg) {
      successMsg.textContent = `"${data.name || 'Sanctuary Display'}" has been successfully authorized and pinned to this console.`;
    }
    if (successDetails) {
      successDetails.innerHTML = `
        <div><strong>Device ID:</strong> ${escapeHtml(data.device_id)}</div>
        <div><strong>Platform:</strong> ${escapeHtml(data.platform || 'TV Client')}</div>
        <div><strong>Token Issued:</strong> <span style="font-family: monospace;">${escapeHtml((data.token || '').slice(0, 14))}…</span></div>
      `;
    }
  }

  // Pair Another Display — reuses the same session token for as many
  // devices as the technician can authorize within its window, rather than
  // minting a fresh one from the phone itself. Minting is console-only now:
  // the phone was never meant to hold the credential that does that, so the
  // session the console handed it via the QR (`?key=...`) simply stays
  // valid across multiple authorizations instead of being single-use — see
  // `validate_pairing_session` in src/api/ws.rs. If it's expired by the
  // time they scan the next TV, `authorize` below will fail plainly and
  // ask them to get a fresh QR from the console.
  if (pairAnotherBtn) {
    pairAnotherBtn.addEventListener('click', () => {
      if (successCard) successCard.style.display = 'none';
      if (scannerCard) scannerCard.style.display = 'block';
      if (placeholderEl) placeholderEl.style.display = 'flex';
      startCamera();
    });
  }
})();
