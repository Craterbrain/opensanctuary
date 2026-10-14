// A sandboxed plugin has no direct access to the console or the server: everything goes through the
// console's message bridge (the SDK's OS.call), which checks each request against what the operator
// granted (docs/PLUGIN_PERMISSIONS.md). Methods: getState, sendCommand, keyring.*, net.fetch,
// library.*, process.*, tool.*, ui.theme, ui.resize, ui.toolbar.update, log. Events: OS.on('state', ...) (show.observe), OS.on('toolbar', ...) (ui.toolbar).
// The page is themed by __os/theme.css, which the console keeps in sync with its own look.
(function () {
  'use strict';

  function render(snapshot) {
    var items = (snapshot && snapshot.schedule && snapshot.schedule.items) || [];
    var blackout = snapshot && snapshot.state && snapshot.state.is_blackout;
    document.getElementById('status').textContent = 'Items: ' + items.length + ' · Blackout: ' + (blackout ? 'yes' : 'no');
  }

  async function send(name) {
    var out = document.getElementById('result');
    try {
      await OS.call('sendCommand', { command: name });
      out.textContent = name + ' sent';
    } catch (e) {
      out.textContent = name + ': ' + e.message;
    }
  }

  document.getElementById('prev').addEventListener('click', function () { send('PrevSlide'); });
  document.getElementById('next').addEventListener('click', function () { send('NextSlide'); });
  OS.on('state', render);
  // The ribbon button declared in plugin.json (ui.toolbar): the console tells us when it is clicked.
  OS.on('toolbar', function (e) { if (e && e.id === 'next') send('NextSlide'); });

  OS.call('getState').then(render).catch(function (e) {
    document.getElementById('status').textContent = e.message;
  });
  OS.call('ui.resize', { height: 120 }).catch(function () {});
})();
