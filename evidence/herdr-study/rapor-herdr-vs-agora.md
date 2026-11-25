# herdr ↔ Agora karşılaştırması: geride olduğumuz yerler

Tarih: 2026-11-25 · Agora: main `4b1b04e1` (0.3.38) · herdr: github.com/herdrdev/herdr (klon adresi ogulcancelik/herdr) HEAD `e8d7d97` (2026-11-20, son sürüm 0.9.5, 2026-10-14)
Yöntem: herdr kaynağı `/tmp/herdr-study/src` altına salt okunur klonlandı (derlenmedi, çalıştırılmadı). Üç ayrı inceleme yapıldı
(herdr çalışma zamanı/terminal/algılama, herdr ağ/API/eklenti/dağıtım, Agora'nın gerçekte yerleşik olanları). Agora tarafı
`docs/capabilities.md`, `docs/backlog/roadmap.md`, CHANGELOG ve kaynakla doğrulandı; kritik iddialar elle tekrar kontrol edildi
(`manifest.rs:420` hooks yok sayılıyor; `manifests/agents/` 3 L1 tür + 42 registry ACP türü; `title` bölgesi motorda var ama hiçbir
manifest kullanmıyor; `agent explain` yok; `agora-platform`da OS kapanış sinyali yok).

## 1. Kısa özet

herdr ile Agora aynı problemi farklı taraftan çözüyor:

- **herdr**: tek kullanıcılı, terminal içinde çalışan (TUI) bir "ajan çoklayıcısı". Mart 2026'dan beri yayında, 60+ sürüm,
  issue/PR sayacı #6107'ye ulaşmış, ~330 bin satır Rust. Son sürüm 2026-10-14; Kasım'da yalnız 4 commit (stabilizasyon). Gücü: **çok sayıda gerçek ajanı doğru tanımak, oturumlarını geri getirmek, terminal kalitesi,
  kurulum/güncelleme kolaylığı ve betiklenebilir API**.
- **Agora**: hub + node + web/masaüstü/TUI/telefon istemcileri, çok kullanıcılı, imzalı onaylar, sandbox, ajanlar arası mesh.
  18 günlük, ~262 bin satır Rust + ~42 bin satır TS. Gücü: **mimari, güvenlik, ekip modu, mesh ve GUI**.

Bizim **en çok geride olduğumuz yer terminal modundaki (L1) ajan kapsamı ve algılama olgunluğu**; sohbet (ACP) modunda
kapsam kâğıt üstünde geniş ama türlerin çoğu denenmemiş; ikinci sırada **dağıtım/kurulum** ve
**güç kullanıcısına dönük terminal/TUI deneyimi**. Mimari, güvenlik ve çok makineli koordinasyonda öndeyiz.

## 2. Karşılaştırma tablosu

| Alan | herdr | Agora | Durum |
|---|---|---|---|
| Terminal modunda tanınan ajan | 24 tür (22 ekran manifesti) | 3 L1 tür (claude-code, codex, wrongstack) | **Gerideyiz** |
| Sohbet (ACP) modunda ajan | yok | 42 registry türü (Claude, Codex, Gemini, Copilot, Cursor, Amp, Kiro, Qwen, OpenCode…); claude-acp canlı çalışıyor, çoğu denenmemiş | Öndeyiz, doğrulama borcu var |
| Ajan kancaları / kendini bildirme | 18 ajan için `integration install`; `pane report-agent` protokolü | `[hooks]` ayrıştırılıyor ama yok sayılıyor | **Gerideyiz** |
| ACP/yapılandırılmış ajanlar | yok | L2 ACP + 41 kayıt türü (denenmemiş) + L3 adaptör | Öndeyiz (kâğıt üstünde) |
| Algılama girdileri | ekran bölgeleri + OSC 0/2 başlık + OSC 9 ilerleme + süreç ağacı + kanca | alt satırlar + `title` bölgesi (kullanılmıyor) + süreç adı; OSC 9 ilerleme yok | Kısmen gerideyiz |
| Algılama hata ayıklama | `agent explain` (canlı ya da dosyadan) | yok | **Gerideyiz** |
| Algılama kurallarını yayınsız güncelleme | herdr.dev'den uzaktan manifest (imzasız) | yalnız yerel override | Gerideyiz (bizimki imzalı olmalı) |
| Oturum devamı (resume) | ~18 ajan | Claude, Codex, ACP; WrongStack fiilen "yeniden başlat" | **Gerideyiz** |
| Süreçler sunucu/node yeniden başlayınca yaşıyor mu | yalnız deneysel live handoff (SCM_RIGHTS) | holder ile Linux/macOS'ta evet; Windows'ta hayır | Öndeyiz (Windows hariç) |
| Kapatma/yeniden başlatmada kayıt | logind, macOS loginwindow, Windows kapanış kancaları; 48 anlık görüntü; bozuk dosya yedeği | node olay günlüğü ve ledger; OS kapanış kancası yok | Kısmen gerideyiz |
| Terminal motoru | libghostty-vt (yamalı, izlenen) | node'da alacritty_terminal, web'de xterm.js | Eşit sayılır |
| Görseller (kitty graphics) | varsayılan açık | yok | **Gerideyiz** |
| Ses / bildirim sesi | done/request sesleri, ajan bazında | yok (zil sadece görsel) | **Gerideyiz** |
| Kaydırma geçmişi | bayt bazlı 10 MB/bölme, boştayken sıkıştırılır | node 2000 satır, web 5000 (en fazla 20k) | Gerideyiz |
| Klavye kopyalama modu, `$EDITOR` ile geçmiş | var | web'de arama var; vi kopyalama modu ve editörde açma yok | Gerideyiz |
| Klavye kısayollarını özelleştirme | tam (prefix, diziler, eklenti aksiyonları) | yok (ADR-0068'in sonraki basamağı) | **Gerideyiz** |
| TUI düzeni (bölme/sekme/popup) | ürünün kendisi | konsol + sekmeler; düzen modeli TUI'de yok | Gerideyiz |
| GUI (web/masaüstü/telefon) | yok (telefondan SSH + dar ekran düzeni) | web + Tauri + PWA | **Öndeyiz** |
| Çok makine | düz SSH federasyonu, daemon yok, uzakta otomatik kurulum | hub + mTLS + eşleştirme + SSH tüneli + Tailscale | Farklı; kurulum kolaylığında gerideyiz |
| Uzak kurulum doğrulaması | gerçek kullanıcılarla, Windows uzak dahil | yalnız loopback sshd ile denendi; Windows'a ssh install yok | Gerideyiz |
| Betiklenebilir API | 102 metot, olay aboneliği + `events_lost`, layout export/apply, üretilmiş JSON Schema | protobuf şema; CLI'da agent start/stop/prompt/list | Gerideyiz (dış betikleme yüzeyi) |
| Ajanlar arası koordinasyon | CLI ile bölme aç, prompt yaz, `wait --until blocked` | mesh: mesaj, ask, görev panosu, lease, handoff, makineler arası | **Öndeyiz** |
| Eklentiler | manifest tabanlı, GitHub'dan kurulum, pazar yeri, link işleyiciler, bölme/popup | arayüz eklentisi yok (yalnız ajan adaptörleri) | **Gerideyiz** |
| Güvenlik / onay | aynı Unix kullanıcısı güveni; onay mekanizması yok | imzalı onaylar, politika, sandbox, sır maskeleme | **Öndeyiz** |
| Ekip / roller / denetim | yok | owner/member/viewer, denetim dışa aktarma | **Öndeyiz** |
| Kurulum kanalları | `curl | sh`, PowerShell, brew, mise, nix, Windows ARM64 | yalnız GitHub release + `agora update` | **Gerideyiz** |
| Sürüm imzalama | yok (yalnız sha256) | SSH imzası + pinlenmiş anahtar | Öndeyiz |
| Stable/preview kanalı | var | yok | Gerideyiz |
| Belge sitesi | sürümlü, en/zh-CN/ja | yok (Q15 açık) | **Gerideyiz** |
| Uygulama içi i18n | yok (arayüz İngilizce) | en + tr, Fluent | Öndeyiz |
| Ölçek | 500+ bölme düzeltmeleri, render benchmark | 50 terminal ölçüldü; 2000 ajanlık ağaç bütçeyi aşıyor | Gerideyiz |
| Gerçek dünya olgunluğu | 8 ay, 60+ sürüm, Homebrew'da | 18 gün, gerçek ajan testleri ince (M1.2 açık) | Gerideyiz |

## 3. Geride olduğumuz yerler, önceliğe göre

### P0: ürünün temel vaadini doğrudan etkileyenler

**G1. Terminal modunda (L1) ajan kapsamı: 3'e karşı 24.**
- herdr ekranla şunları tanıyor: Claude Code, Codex, Gemini CLI, Copilot CLI, Cursor Agent, OpenCode, Amp, Droid, Devin, Kimi,
  Kilo, Kiro, Qwen, Qoder, Grok, Hermes, Letta, Antigravity, Cline, Pi, OMP, MastraCode, Maki, Muse (`src/detect/manifests/*.toml`).
- Agora'da terminal modu manifesti yalnız `claude-code`, `codex` ve `wrongstack` için var.
- Sohbet modunda ise `manifests/agents/registry/` altında 42 ACP türü var ve herdr listesinin büyük kısmını kapsıyor
  (claude-acp, codex-acp, gemini, github-copilot-cli, cursor, amp-acp, kiro, kimi, qwen-code, qoder, kilo, devin,
  factory-droid, cline, opencode, pi-acp, grok-build…).
  - `claude-acp` şu an üç makinede canlı çalışıyor.
  - Diğerleri ADR-0092 gereği "Untested". ACP üzerinden onay ve durum ajanın kendisinden geliyor, ekran algılaması gerekmiyor.
- Yani boşluk iki parça:
  1. ajanını gerçek terminal arayüzüyle (TUI) kullanmak isteyen için L1 manifest eksik;
  2. ACP türlerinin gerçek programla doğrulanması eksik.
- Öneri:
  - en çok kullanılan 5–6 ACP türünü gerçek programla çalıştırıp "Untested" etiketini kaldıralım (Gemini, Copilot, Cursor,
    OpenCode, Qwen);
  - terminal modu için en az Gemini CLI ve OpenCode L1 manifesti ekleyelim (M1.2).
  - herdr manifestleri Apache-2.0; kalıplar referans olarak okunabilir, kopyalanan kısım olursa atıf gerekir.

**G2. Algılama motorunda herdr'daki olgunluk katmanları eksik.**
Agora'nın motorunda zaten şunlar var (`docs/specs/agent-manifest.md`, `crates/agora-adapters/src/rules.rs`):
- `title` / `bottom_lines(n)` / `bottom_non_empty_lines(n)` bölgeleri;
- öncelik ve dosya sırasıyla eşitlik bozma;
- `all` / `any` / `not` koşulları;
- uyarlamalı tarama hızı (`Pacer`);
- "eşleşme yoksa görüş yok" kuralı;
- `[[ready]]` / `[[typed]]`.

herdr'da olup bizde olmayanlar:
- **Başlık kuralı yazılmamış.** Motor `title` bölgesini destekliyor ama dağıtılan hiçbir manifest kullanmıyor.
  - herdr'da Claude Code çalışırken başlıkta Braille spinner, boştayken `✳ ` gösteriyor ve bu en yüksek öncelikli kural
    (`claude.toml`). Codex'te de blocked/working/idle başlıktan okunuyor.
  - En ucuz kazanç: claude-code ve codex manifestlerine başlık kuralları ve fixture'ları eklemek. Motor değişikliği gerekmiyor.
- **OSC 9;4 ilerleme girdisi yok** (Claude boşta `4;0` gönderiyor). Yeni bölge türü gerektirir.
- **Giriş kutusuna göre bölgeler** (`prompt_box_body`, `above_prompt_box`, `after_last_horizontal_rule`, Codex için
  `after_last_prompt_marker`): eski onay metninin yanlış "blocked" üretmesini engelliyor.
  - Bunlar "yalnız canlı alt kısım" kuralıyla uyumlu.
  - herdr'ın `whole_recent` ve `top_non_empty_lines` bölgeleri ise bu kuralla çelişebilir; onları almayalım.
- **`skip_state_update`**: transkript görüntüleyici veya model seçici açıkken durum değişmiyor, donuyor.
- **Zamanlama korumaları**: working→idle geçişi 3×100 ms onayla geciktiriliyor; blocked her 800 ms yeniden yayınlanıyor;
  açılışta 3 s tolerans var. Agora'da böyle bir debounce bulamadım (`agora-node` ve `agora-adapters` içinde arandı).
- **`herdr agent explain`**: hangi kural, hangi bölge, hangi kanıt eşleşti, canlı ya da kayıtlı ekrandan gösteriyor.
  Agora'da karşılığı yok.
- **Yayınsız kural güncelleme**: herdr imzasız uzak manifest çekiyor. Biz yaparsak imzalı olmalı (adaptör kaydındaki Ed25519
  modeli gibi).
- Öneri sırası:
  1. başlık kuralları (yalnız manifest, ADR gerekmez);
  2. `agora agent explain`;
  3. debounce;
  4. yeni bölge türleri ve `skip_state_update` (manifest şeması değiştiği için ADR).

**G3. Kanca (hook) ve kendini bildirme protokolü yok.**
- herdr'da `herdr integration install claude|codex|…` 18 ajanın config'ine kanca yazıyor. Ayrıca herhangi bir ajan
  `"$HERDR_BIN_PATH" pane report-agent --source --agent --state --seq [-- <resume argv>]` ile durumunu ve devam komutunu
  bildirebiliyor; herdr'a PR açmak gerekmiyor (add-herdr-support.mdx).
  - Sıra numarası ile eski raporlar reddediliyor.
  - Ekranda görünen bir onay iletisi bayat bir "working" kancasını geçersiz kılıyor.
- Agora'da `[hooks]` bölümü ayrıştırılıyor ama yok sayılıyor (`crates/agora-adapters/src/manifest.rs:420`, P4-05 "Not done: hooks").
- Öneri: L1.5 seviyesi olarak node kontrol soketi üzerinden `agora agent report` tasarlanmalı. Mesh güvenilmez veri kuralına
  uymalı: bildirim yalnız durum ve oturum kimliği taşır, yetki taşımaz. Ajan entegrasyonu artık çekirdek değişikliği
  gerektirmez. ADR-0094 ile birlikte düşünülmeli.

**G4. Oturum devamı (resume) kapsamı.**
- herdr yaklaşık 18 ajanı yeniden başlatmada kendi konuşmasına döndürüyor (`agent_resume.rs`, session-state.mdx):
  - devam komutunu gerçek bir kabuğa yazıyor, ajan çıkınca bölme kabuğa düşüyor;
  - host temasını öğrenene kadar bekliyor;
  - ajanları 100 ms arayla başlatıyor;
  - iç içe veya geçici alt oturumların (`claude -p`) sahibin oturumunu ezmesini engelliyor.
- Agora'da Claude ve Codex için var; WrongStack'in `[session]` bölümü olmadığı için pratikte yalnız "Start again" çalışıyor
  (`ledger.rs:455`).
- Öneri: G1'deki her yeni manifest `[session]` ve `[resume]` ile gelsin; WrongStack'e `[session]` eklensin.

**G5. Kurulum ve dağıtım kanalları.**
- herdr'ın kanalları:
  - `curl -fsSL https://herdr.dev/install.sh | sh`, PowerShell yükleyici ve uç nokta koruması için `install.cmd`;
  - Homebrew, mise ve Nix flake; musl statik Linux ikilileri; Windows ARM64;
  - stable/preview kanalı;
  - paket yöneticisi farkındalığı: brew ile kurulduysa "brew upgrade" diyor.
- Agora'da yalnız GitHub release + `agora update` var. Brew, winget, scoop, install script, preview kanalı yok. Windows imzası
  (ADR-0077) de bekliyor.
- Öneri:
  1. imza doğrulayan bir `install.sh` / `install.ps1` (bizim pinlenmiş anahtarımızla; herdr'da yalnız sha256 var, burada öne geçeriz);
  2. Homebrew tap;
  3. preview kanalı (main build'leri).
  Kurulum isteyen kullanıcı için en büyük sürtünme bu.

### P1: güç kullanıcısı deneyimi

**G6. Terminal yeteneği eksikleri.**
- Kitty graphics veya başka görsel protokol yok. herdr'da varsayılan açık; ajanlar görsel/diyagram çıktısı veriyor.
  xterm.js'in image addon'u (sixel/iTerm) en azından web'de ucuz bir başlangıç olur.
- **Ses yok.** herdr'da "bitti" ve "onay istiyor" sesleri var, ajan bazında açılıp kapanabiliyor. Agora'da zil yalnız görsel.
  Bir ajanın onay beklediğini başka sekmedeyken duymak temel bir ihtiyaç.
- Kaydırma geçmişi: herdr bölme başına 10 MB (bayt bazlı) tutuyor ve boşta sıkıştırıyor. Bizde node 2000 satır, web varsayılan
  5000 satır. Uzun ajan çıktılarında geçmiş kayboluyor.
- Klavye ile vi tarzı kopyalama modu, "geçmişi `$EDITOR`'da aç" ve smart-case arama TUI'de yok.
- Alternatif ekrandaki ajanın (Claude Code, OpenCode) geçmişini okuma: herdr sentetik tekerlek olaylarıyla sayfa sayfa
  topluyor, sonra görünümü geri koyuyor (`alt_screen_read.rs`). Bizim `agent_output` ekrandan ya da (yapılandırılmış ajanlarda) transkriptten en fazla 200 satır veriyor (`mesh-tools.md`); terminal ajanının alternatif ekran geçmişi yok.
- Host renklerine yanıt: OSC 10/11/4 sorgularına gerçek host renkleriyle cevap veriyor ve mode 2031 ile açık/koyu bildiriyor.
  Bizde web teması ile ajanın gördüğü renkler eşleşiyor mu, kontrol edilmeli.

**G7. Klavye kısayolları ve TUI düzeni.**
- herdr'da:
  - özelleştirilebilir prefix (`ctrl+b`, birden fazla da olabilir) ve her eylem bağlanabilir;
  - navigate modu, Go To seçici (`prefix+g`), popup terminaller, kalıcı bölme/sekme/çalışma alanı;
  - fare ile sürükle-böl, sekme sürükleme.
- Agora'da kısayollar sabit (ADR-0068'in "keybindings" basamağı yapılmadı). TUI'de düzen modeli yok (M2.11).
- Öneri: önce web'de kısayol özelleştirme, sonra TUI'ye bölme/sekme. Bu bir UI işi; ADR-0024 kuralı geçerli, önce mockup.

**G8. Betiklenebilir API ve CLI yüzeyi.**
- herdr:
  - 102 metot; `events.subscribe` ve 512'lik halka ile geride kalan aboneye açık `events_lost`; `session.snapshot` ile uzlaştırma tarifi;
  - `layout export/apply`; `pane wait-output --regex`;
  - `agent prompt --wait`: atomik, onay bekleyen ajana yazmayı reddediyor, 5 s içinde iş başlamazsa `agent_prompt_stalled` dönüyor;
  - `agent start` ajan gerçekten hazır olunca dönüyor;
  - üretilmiş JSON Schema ikilinin içinde (`herdr api schema`);
  - `--machine <ad>` ile her komut uzak makinede çalışıyor.
- Agora'da CLI'da `agent start/stop/prompt/list/resume` var. Olay aboneliği, düzen uygulama ve çıktı bekleme CLI'dan yok.
  `agent prompt` onay bekleyen ajanı reddediyor mu, bilmiyoruz; kontrol edilmeli.
- Öneri:
  1. `agora agent prompt` için "blocked ise reddet" ve "--wait";
  2. `agora agent wait --until blocked|idle`;
  3. `agora events` (NDJSON akışı).
  Protokolde yeni mesaj gerekiyorsa ADR.

**G9. Eklenti sistemi ve pazar yeri.**
- herdr'da `herdr-plugin.toml` var:
  - aksiyonlar, olay kancaları, bölme/popup, link işleyicileri, başlangıç kancaları;
  - GitHub'dan kurulum, kurulumdan önce güven önizlemesi;
  - değişmez kurulum dizinleri ve lease ile çöp toplama;
  - herdr.dev/plugins pazar yeri;
  - "CLI'ın tamamı eklenti API'si" yaklaşımı.
- Agora'da arayüz eklentisi yok (ADR-0068 panel kaydı yapılmadı); L3 adaptörleri yalnız ajan entegrasyonu için.
- Not: herdr eklentileri sandbox'sız çalışıyor. Biz yaparsak imzalı onay ve politika modelimizle daha güvenli yapabiliriz.
  Önce ADR gerekir.

**G10. Çok makine: kurulum sürtünmesi ve gerçek dünya testi.**
- herdr düz OpenSSH ile çalışıyor; daemon ya da port gerekmiyor. `machine add` sırasında:
  - uzak işletim sistemini ve mimariyi tespit ediyor (Windows dahil);
  - varsayılan cevabı "Hayır" olan bir onayla doğru ikiliyi sha256 doğrulayarak kuruyor;
  - ControlMaster paylaşıyor, MFA/Tailscale girişi için `machine reconnect` sunuyor;
  - kalıcı bir `SSH_AUTH_SOCK` ile yeniden bağlanmalarda git imzalamayı koruyor;
  - panodaki görseli uzak makineye geçici dosya olarak aktarıyor.
- Bağlantı kesilince son durum soluk gösteriliyor ve girdi engelleniyor. Yeniden bağlanma geri çekilme (backoff) ve uykudan
  uyanma farkındalığıyla yapılıyor.
- Agora'da `hub ssh add/install` var ama yalnız loopback sshd ile denendi, Windows hedefe ssh install yok. Tailscale ve
  Cloudflare gerçek ortamda hiç çalıştırılmadı.
- Öneri:
  1. blackview → mac/win gerçek `hub ssh install` denemesi;
  2. uzak ajana görsel yapıştırma;
  3. `SSH_AUTH_SOCK` sürekliliği (git imzası).

**G11. Kapanışa dayanıklı kayıt ve anlık görüntü geçmişi.**
- herdr'ın kaydı:
  - logind delay-lock (Linux), macOS loginwindow notify anahtarları (ajanlar öldürülmeden 15–20 s önce kayıt) ve Windows
    kapanış kancası;
  - fsync + rename + dizin fsync;
  - 48 adet 15 dakikalık düzen anlık görüntüsü;
  - yüklenemeyen dosyanın yedeği.
- Agora'da düzen tarayıcı tarafında; node ledger'ı var ama OS kapanış sinyali dinlenmiyor. Windows'ta holder yok (M2.6),
  ajanlar node ile ölüyor.
- Öneri: `agora-platform` içinde kapanış ve uyku sinyalleri (kural gereği yalnız o crate'te), ledger'ın kapanıştan önce
  flush edilmesi.

### P2: olgunluk ve cila

**G12. Belge sitesi.** herdr'ın sitesi sürümlü (0.5.11 → 0.9.5 anlık görüntüleri), en/zh-CN/ja dillerinde ve CI'da çeviri
eşitliği kontrolü var. `herdr --skill` ile ajanlara sürümle eşleşen bir beceri dosyası veriyor. Agora'da site yok (Q15).

**G13. Bildirim çeşitliliği.**
- herdr'da:
  - OSC 9/99 ile host terminale bildirim (Ghostty, iTerm2, Kitty, WezTerm);
  - macOS'ta terminal-notifier;
  - Windows'ta tıklayınca ilgili bölmeye gitme;
  - kenar çubuğunda "done" rozeti (görülmemiş tamamlanma, `completion_seq`).
- Agora'da ntfy ve webhook var (öndeyiz), ama masaüstü kabuğunda macOS bildirimi yok ve tıklayınca pencere odaklanmıyor.

**G14. Ölçek.**
- herdr'ın ölçek çalışmaları:
  - 500+ bölme için pidfd ile süreç toplama;
  - yalnız seçili makinenin ekran akıtması (`surface_interest`);
  - satır kaydırma ve değişen satır yamaları;
  - tek yuvalı render şeridi: yavaş istemci birikmez, en yeni kareyi alır;
  - gizli bölme çıktısının render'ı uyandırmaması.
- Bizde 50 terminal ölçüldü; 2000 ajanlık ağaç kaydırma bütçesini aşıyor; çok bölmeli gerçek tarayıcı ölçümü yok (M1.3).

**G15. Kenar çubuğu ve görünüm sorguları.** Metadata token'ları (TTL, sıra numarası, `$ad` ile satıra basma), ajan görünüm
sorguları (filtre ve sıralama), "symbols" durum gösterimi. Agora'da filtre var mı, kontrol edilmeli. UI dili kuralları
nedeniyle doğrudan alınmamalı.

**G16. Gerçek dünya kanıtı.** herdr 8 aydır yayında ve 60+ sürüm çıkarmış; CHANGELOG'u gerçek terminal ve ajan
tuhaflıklarıyla dolu (IME, split OSC yanıtları, Windows ConPTY yapıştırma sınırları). Agora'nın gerçek ajanla testleri ince.
CHANGELOG'u tarayıp "bizde de olur mu" listesi çıkarmak ucuz ve değerli.

## 4. Önde olduğumuz yerler (denge için)

- **GUI**: web, masaüstü ve telefon istemcileri. herdr'da yalnız TUI var.
- **Güvenlik**:
  - imzalı onaylar ve "hub politikayı genişletemez" kuralı;
  - Landlock/sandbox-exec ile sandbox;
  - canlı sır maskeleme;
  - sürüm imzası.
  herdr'da güven modeli "aynı Unix kullanıcısı"; eklentiler ve uzaktan manifestler imzasız.
- **Ekip modu**: roller, proje izinleri, denetim kaydı.
- **Mesh**:
  - mesaj, ask ve kanallar;
  - makineler arası tek yazarlı görev panosu;
  - lease ve handoff;
  - kullanıcı onaylı yardımcı ajan başlatma.
  herdr'da yalnız "bölmeye yaz ve bekle" var.
- **Yapılandırılmış ajanlar**: ACP/L3, kullanım ve kota takibi.
- **Süreçlerin node yeniden başlamasından sağ çıkması** (Linux/macOS holder). herdr'da bu yalnız deneysel live handoff ile oluyor.
- **Çevrimdışı tamponlama**: node hub yokken çalışmaya devam ediyor, olayları sonra sırayla gönderiyor.
- **Uygulama içi Türkçe** (herdr arayüzü yalnız İngilizce).

## 5. Önerilen sıra

Mevcut "tek seferde bir PR" kuralına uygun sıra:

1. **Claude/Codex manifestlerine başlık kuralları** (G2'nin ADR gerektirmeyen kısmı) ve `agora agent explain`.
2. **ACP türlerini gerçek programla doğrulama** (G1): Gemini, Copilot, Cursor, OpenCode, Qwen.
3. **Algılama motoru ADR'si** (G2): debounce, OSC ilerleme, giriş kutusu bölgeleri, `skip_state_update`.
4. **Yeni L1 manifestler** (G1 + G4): Gemini CLI, OpenCode; fixture ve `[session]`/`[resume]` ile. WrongStack'e `[session]`.
5. **Kendini bildirme protokolü + Claude/Codex kancaları** (G3), ADR ile.
6. **İmza doğrulayan install.sh/ps1 + Homebrew tap + preview kanalı** (G5).
7. **Ses ve kalıcı "bitti" rozeti** (G6, G13): küçük iş, büyük etki.
8. **CLI orkestrasyonu**: `agent prompt --wait`, blocked'ı reddetme, `agent wait --until`, `agora events` (G8).
9. **Kısayol özelleştirme** (G7) ve web'de görsel desteği (G6).
10. **Gerçek SSH kurulum denemesi** (G10) ve **OS kapanış kancaları** (G11).
11. **Eklenti ADR'si** (G9) ve **belge sitesi kararı** (G12, Q15).

## 6. Not

herdr Apache-2.0 lisanslı (`LICENSE` doğrulandı). Tasarımları inceleyip kendi koşullarımıza göre yeniden yazmak serbest. Doğrudan kod veya manifest
kalıbı kopyalanırsa lisans atfı (NOTICE) gerekir. Bu raporda herdr'dan kod alınmadı.

## Kaynaklar

- herdr deposu: https://github.com/herdrdev/herdr (klon: https://github.com/ogulcancelik/herdr)
- herdr belgeleri: https://herdr.dev/docs/
- İnceleme: https://dev.to/andrew-ooo/herdr-review-the-agent-multiplexer-your-terminal-needed-li1
- Agora: `docs/capabilities.md`, `docs/backlog/roadmap.md`, `docs/specs/agent-manifest.md`, `docs/specs/mesh-tools.md`, `manifests/agents/`
