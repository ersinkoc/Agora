// The shell's only screen (ADR-0048): choose a hub, confirm its certificate's fingerprint once, then open the hub's page. All text is
// in the locale tables below until the shell reads locales/{en,tr}/desktop.ftl at build time.
const T = {
  en: {
    title: 'Connect to a hub',
    intro: 'Enter the address of your hub and paste its CA certificate (the file ca.pem in the hub folder). The app will trust this certificate and nothing else for this hub.',
    address: 'Hub address', ca: 'CA certificate',
    confirm: 'Check that this fingerprint matches the one printed by “agora hub fingerprint” on the hub, then connect.',
    connect: 'Connect', failed: 'Could not connect. Check the address and the certificate.', badca: 'That is not a certificate.',
    unreachable: 'The hub did not answer. Check the address and that the hub is running.',
    untrusted: 'The hub at this address is not signed by this certificate. Check that you pasted the CA of this hub.',
  },
  tr: {
    title: 'Bir hub’a bağlan',
    intro: 'Hub adresini girin ve CA sertifikasını (hub klasöründeki ca.pem dosyası) yapıştırın. Uygulama bu hub için yalnızca bu sertifikaya güvenir.',
    address: 'Hub adresi', ca: 'CA sertifikası',
    confirm: 'Bu parmak izinin hub’da “agora hub fingerprint” ile yazdırılanla aynı olduğunu kontrol edin, sonra bağlanın.',
    connect: 'Bağlan', failed: 'Bağlanılamadı. Adresi ve sertifikayı kontrol edin.', badca: 'Bu bir sertifika değil.',
    unreachable: 'Hub yanıt vermedi. Adresi ve hub’ın çalıştığını kontrol edin.',
    untrusted: 'Bu adresteki hub bu sertifikayla imzalanmamış. Bu hub’ın CA sertifikasını yapıştırdığınızı kontrol edin.',
  },
};
const t = T[(navigator.language || 'en').toLowerCase().startsWith('tr') ? 'tr' : 'en'];
for (const el of document.querySelectorAll('[data-t]')) el.textContent = t[el.dataset.t];

const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const fail = (msg) => { $('error').textContent = msg; $('error').hidden = false; };
// The shell's `connect` answers with a stable code; anything else is the general message.
const reason = (e) => t[e === 'unreachable' || e === 'untrusted' ? e : 'failed'];

async function open(hub, ca, port) {
  const url = await invoke('connect', { hub, caPem: ca, port });
  window.location.href = url;
}

// What the user has typed so far decides whether the fingerprint is shown; it is shown before anything connects.
let shown = false;
async function showPrint() {
  try {
    $('print').textContent = await invoke('ca_fingerprint', { caPem: $('ca').value });
    $('print').hidden = $('confirm').hidden = false;
    $('error').hidden = true;
  } catch { $('print').hidden = $('confirm').hidden = true; }
}
$('ca').addEventListener('input', showPrint);

$('form').addEventListener('submit', async (e) => {
  e.preventDefault();
  shown = !$('print').hidden;
  if (!shown) return fail(t.badca);
  try { await open($('hub').value.trim(), $('ca').value, null); } catch (err) { fail(reason(err)); }
});

// A hub chosen before opens again without asking. If it cannot be reached now, the form is filled in with it (fingerprint shown, so
// Connect works) and says why.
invoke('saved').then((s) => {
  if (s) open(s.hub, s.ca_pem, s.port).catch(async (err) => {
    $('hub').value = s.hub; $('ca').value = s.ca_pem;
    await showPrint();
    fail(reason(err));
  });
});
