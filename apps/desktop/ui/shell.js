// The shell's only screen (ADR-0048): choose a hub, confirm its certificate's fingerprint once, then open the hub's page. All text is
// in the locale tables below until the shell reads locales/{en,tr}/desktop.ftl at build time.
const T = {
  en: {
    title: 'Connect to a hub',
    intro: 'Enter the address of your hub and paste its CA certificate (the file ca.pem in the hub folder). The app will trust this certificate and nothing else for this hub.',
    address: 'Hub address', ca: 'CA certificate',
    confirm: 'Check that this fingerprint matches the one printed by “agora hub fingerprint” on the hub, then connect.',
    connect: 'Connect', failed: 'Could not connect. Check the address and the certificate.', badca: 'That is not a certificate.',
  },
  tr: {
    title: 'Bir hub’a bağlan',
    intro: 'Hub adresini girin ve CA sertifikasını (hub klasöründeki ca.pem dosyası) yapıştırın. Uygulama bu hub için yalnızca bu sertifikaya güvenir.',
    address: 'Hub adresi', ca: 'CA sertifikası',
    confirm: 'Bu parmak izinin hub’da “agora hub fingerprint” ile yazdırılanla aynı olduğunu kontrol edin, sonra bağlanın.',
    connect: 'Bağlan', failed: 'Bağlanılamadı. Adresi ve sertifikayı kontrol edin.', badca: 'Bu bir sertifika değil.',
  },
};
const t = T[(navigator.language || 'en').toLowerCase().startsWith('tr') ? 'tr' : 'en'];
for (const el of document.querySelectorAll('[data-t]')) el.textContent = t[el.dataset.t];

const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const fail = (msg) => { $('error').textContent = msg; $('error').hidden = false; };

async function open(hub, ca, port) {
  const url = await invoke('connect', { hub, caPem: ca, port });
  window.location.href = url;
}

// What the user has typed so far decides whether the fingerprint is shown; it is shown before anything connects.
let shown = false;
$('ca').addEventListener('input', async () => {
  try {
    $('print').textContent = await invoke('ca_fingerprint', { caPem: $('ca').value });
    $('print').hidden = $('confirm').hidden = false;
    $('error').hidden = true;
  } catch { $('print').hidden = $('confirm').hidden = true; }
});

$('form').addEventListener('submit', async (e) => {
  e.preventDefault();
  shown = !$('print').hidden;
  if (!shown) return fail(t.badca);
  try { await open($('hub').value.trim(), $('ca').value, null); } catch { fail(t.failed); }
});

// A hub chosen before opens again without asking.
invoke('saved').then((s) => { if (s) open(s.hub, s.ca_pem, s.port).catch(() => { $('hub').value = s.hub; $('ca').value = s.ca_pem; }); });
