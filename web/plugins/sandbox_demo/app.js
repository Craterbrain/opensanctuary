// A sandboxed plugin has no direct access to the console or the server: everything goes through
// postMessage to the console's bridge, which checks each request against what the operator
// granted (docs/PLUGIN_PERMISSIONS.md). Methods: getState, sendCommand, keyring.get|set|delete,
// ui.resize, log. Events: os.event { event: 'state', data } (needs show.observe).
const pending = new Map();
let nextId = 0;

function call(method, params) {
  return new Promise((resolve, reject) => {
    const id = ++nextId;
    pending.set(id, { resolve, reject });
    parent.postMessage({ type: 'os.request', id, method, params }, '*');
  });
}

window.addEventListener('message', (ev) => {
  if (ev.source !== parent) return;
  const m = ev.data;
  if (!m) return;
  if (m.type === 'os.response' && pending.has(m.id)) {
    const p = pending.get(m.id);
    pending.delete(m.id);
    m.ok ? p.resolve(m.result) : p.reject(new Error(m.error));
  } else if (m.type === 'os.event' && m.event === 'state') {
    render(m.data);
  }
});

function render(snapshot) {
  const items = (snapshot && snapshot.schedule && snapshot.schedule.items) || [];
  const blackout = snapshot && snapshot.state && snapshot.state.is_blackout;
  document.getElementById('status').textContent = `Items: ${items.length} · Blackout: ${blackout ? 'yes' : 'no'}`;
}

async function send(name) {
  const out = document.getElementById('result');
  try {
    await call('sendCommand', { command: name });
    out.textContent = `${name} sent`;
  } catch (e) {
    out.textContent = `${name}: ${e.message}`;
  }
}

document.getElementById('prev').addEventListener('click', () => send('PrevSlide'));
document.getElementById('next').addEventListener('click', () => send('NextSlide'));

call('getState').then(render).catch((e) => {
  document.getElementById('status').textContent = e.message;
});
call('ui.resize', { height: 120 }).catch(() => {});
