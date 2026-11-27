# Komut satırı çıktısı. Makine tarafından okunan veriler (kimlikler, durumlar, yollar) olduğu gibi yazılır; yalnızca cümleler burada.

cli-usage =
    Kullanım:
      agora [--ui tui|web|none [--save]] [--resume ask|auto [--save]]
      agora web
      agora status
      agora down [--agents]
      agora service install|remove|status
      agora node run [--socket YOL] [--manifests DİZİN] [--project YOL]... [--data DİZİN] [--worktrees DİZİN] [--resume auto]
      agora agent start TÜR [--cwd YOL] [--worktree] [--project AD] [--name AD] [--setting KİMLİK=DEĞER]... [--accept-project-id-change] [--socket YOL]
      agora project init [--cwd YOL] [--socket YOL]
      agora agent stop AJAN_KİMLİĞİ [--socket YOL]
      agora agent prompt AJAN_KİMLİĞİ METİN... [--wait [--timeout SANİYE]] [--force] [--socket YOL]
      agora agent wait AJAN_KİMLİĞİ [--until working|blocked|idle|done|exited[,...]] [--timeout SANİYE] [--socket YOL]
      agora events [--agent AJAN_KİMLİĞİ] [--json] [--socket YOL]
      agora agent resume AJAN_KİMLİĞİ [--again] [--socket YOL]
      agora agent list [--socket YOL]
      agora agent kinds [--socket YOL]
      agora agent explain TÜR --file EKRAN [--size SÜTUNxSATIR] [--manifests KLASÖR]
      agora terminal attach AJAN_KİMLİĞİ [--socket YOL]
      agora node stdio [--socket YOL]
      agora mesh stdio --socket YOL
      agora hub run [--listen ADDR] [--hub-dir DIR] [--name HOST]... [--web DIR] [--metrics] [--preview-ports FIRST-LAST]
      agora hub code [--hub-dir DIR]
      agora hub user add NAME [--role owner|member|viewer] | list | role ID ROLE | remove ID | login ID [--hub-dir DIR]
      agora hub user projects | grant|revoke ID MACHINE/PROJECT | grant|revoke ID MACHINE FOLDER [--hub-dir DIR]
      agora hub fleet on|off|status [--hub-dir DIR]
      agora hub remote-helpers on|off|status [--hub-dir DIR]
      agora hub task-review enable PROJECT_ID [--hub-dir DIR]
      agora hub task-review notify-parent PROJECT_ID [--hub-dir DIR]
      agora hub plugin add FILE|list|remove ID [--hub-dir DIR]
      agora hub audit export [--since MS] [--who ID] [--hub-dir DIR]
      agora hub metrics-token create NAME [--expires DAYS] | list | revoke NAME [--hub-dir DIR]
      agora hub ssh install AD [--yes] [--unsigned] [--release-url URL] [--hub-dir DIR]
      agora hub login [--hub-dir DİZİN]
      agora password set|clear|status [--hub-dir DİZİN]
      agora remote tailscale [--serve] [--port N] | status | off [--hub-dir DIR]
      agora hub notify [--ntfy URL | --webhook URL | --test] [--hub-dir DIR]
      agora hub prices update [--url URL] | set MODEL GİRİŞ ÇIKIŞ [ÖNBELLEK_OKUMA [ÖNBELLEK_YAZMA]] | unset MODEL [--hub-dir DİZİN]
      agora hub prices auto on [--url URL] | auto off | show [--hub-dir DİZİN]
      agora hub themes on|off|status | update [--url URL] [--hub-dir DIR]
      agora hub plugins on|off|status | update [--url URL] | key set FILE | install ID [--yes] | upgrade ID [--yes] [--hub-dir DIR]
      agora hub ssh [add AD HEDEF [--port N]] [--hub-dir DIR]
      agora hub backup [DOSYA] [--keep N] [--hub-dir DİZİN]
      agora hub restore DOSYA [--hub-dir DİZİN]
      agora hub machines [forget AD | forget --old] [--refuse-return] [--yes] [--hub-dir DİZİN]
      agora hub devices [forget CİHAZ_ID] [--hub-dir DİZİN]
      agora pair open [--minutes N] | close | status [--hub-dir DIR]
      agora join [ADDRESS] [--id NAME] [--replace]
      agora node enroll --hub URL --ca FILE --code CODE --id NAME [--yes --fingerprint PARMAK_İZİ]... [--include-others] [--replace]
      agora panel
      agora node trust CİHAZ_ID AÇIK_ANAHTAR_HEX [--data DİZİN]
      agora node trust --from-hub [--yes --fingerprint PARMAK_İZİ]... [--include-others] [--replace] [--data DİZİN]
      agora node untrust CİHAZ_ID [--data DİZİN]
      agora node secret set AD | list | remove AD [--data DİZİN]
      agora history purge [--data DİZİN]
      agora workspaces [--data DİZİN] [--worktrees DİZİN] [--socket YOL]
      agora workspaces clean [--older-than 7d] [--dry-run] [--data DİZİN] [--worktrees DİZİN] [--socket YOL]
      agora tui [--hub ADRES:PORT --ca DOSYA --code KOD] [--hub-dir DİZİN]
      agora update [--check] [--version X] [--release-url URL] [--release-key DOSYA] [--unsigned]
      agora adapter install AD [--registry URL] [--root-key DOSYA] [--yes] | update [AD] | list | remove AD
      agora agent install AD --list DOSYA [--root-key HEX|DOSYA] [--go-back]
      agora plugin new KİMLİK [KLASÖR] | check KLASÖR|DOSYA | pack KLASÖR [--out DOSYA] [--write-hashes] [--watch]
      agora doctor [--data DİZİN] [--manifests DİZİN] [--socket YOL]
      agora manual [KONU] [--json]
cli-node-listening = Düğüm çalışıyor. Denetim soketi: { $socket }
cli-node-events-set-aside = Bu makine { $node_id } olarak bağlanmadan önce ({ $old_id } adıyla) kaydedilen olaylar bir kenara ayrıldı ve hub'a gönderilmeyecek: { $path }
cli-agent-started = { $agent_id } ajanı { $worktree } içinde başlatıldı
cli-agent-stopped = { $agent_id } ajanı durduruldu
cli-agent-prompted = İstem { $agent_id } ajanına gönderildi
cli-agent-prompt-blocked-hint = Önce ajana yanıt verin ya da metni yine de yazmak için --force ekleyin.
cli-agent-wait-bad-state = --until virgülle ayrılmış durumlar alır: working, blocked, idle, done, exited.
cli-agent-wait-blocked = { $agent_id } ajanı sizi bekliyor: bir yanıt bekliyor.
cli-agent-wait-exited = { $agent_id } ajanı artık çalışmıyor (son durumu: { $state }).
cli-agent-wait-timeout = { $agent_id } ajanı { $seconds } sn beklendi, bekleme bırakıldı; durumu: { $state }.
cli-event-present = { $agent_id } ({ $kind }): { $state }
cli-event-added = { $agent_id } ({ $kind }) göründü: { $state }
cli-event-state = { $agent_id }: { $previous } → { $state }
cli-event-exited = { $agent_id } durdu (son durumu: { $state })
cli-event-running = { $agent_id } yeniden çalışıyor: { $state }
cli-event-restarted = { $agent_id } yeniden başlatıldı (şimdiye dek { $restarts } kez)
cli-event-interrupted = { $agent_id } yarıda kaldı (son durumu: { $state })
cli-event-removed = { $agent_id } artık listede yok
cli-agent-resumed = { $agent_id } ajanı { $worktree } içinde devam ettirildi
cli-agent-started-again = { $agent_id } ajanı { $worktree } içinde yeniden başlatıldı (yeni bir konuşma)
cli-agent-list-empty = Bu düğümde ajan yok.
cli-agent-kinds-empty = Bu düğümde ajan tanımı yok.
cli-terminal-detached = { $agent_id } ajanından ayrıldınız.
cli-attach-needs-terminal = Bağlanmak için etkileşimli bir terminal gerekir.
cli-node-not-running = { $socket } adresinde dinleyen bir düğüm yok. "agora node run" ile başlatın.
cli-node-refused = { $socket } adresindeki düğüm bu süreci reddetti. Düğüm yalnızca kendi kullanıcısını kabul eder, sınırlandırılmış bir ajanı asla; komutu düğümü başlatan kullanıcıyla çalıştırın.
cli-node-needs-token = Düğüm, yalnızca onu başlatan kullanıcının okuyabildiği denetim belirtecini istiyor ve bu işlem bir belirteç sunamadı. Kısıtlanmış bir ajan düğümün denetim soketini kullanamaz.
cli-unknown-project = Bu düğümde { $project } adında kayıtlı proje yok.
cli-ambiguous-project = Birden fazla proje kayıtlı; --project ile birini seçin.
cli-no-projects = Bu düğümde proje yok. --project YOL ile başlatın.
cli-plugin-usage = Kullanım: agora plugin new KİMLİK [KLASÖR] | check KLASÖR|DOSYA | pack KLASÖR [--out DOSYA] [--write-hashes] [--watch]
cli-plugin-new-done = { $id } eklentisi { $folder } içinde oluşturuldu. Sonra: "agora plugin pack { $folder } --out { $id }.json --write-hashes --watch", ardından pencerede Eklentiler, Dosyadan yükle… ile { $id }.json dosyasını seçin. Ayrıntılar { $folder }/README.md içinde.
cli-plugin-new-bad-id = "{ $id }" bir eklenti kimliği olamaz: küçük harf, rakam ve "-" kullanın (en fazla 40, "-" ile başlamadan).
cli-plugin-new-exists = { $folder } zaten var; hiçbir şey yazılmadı. Başka bir klasör seçin.
cli-plugin-write-failed = { $path } yazılamadı: { $reason }
cli-plugin-ok = { $path } pencerenin denetimlerinden geçiyor: { $name } { $version } ({ $id }), { $panels } panel, görmek istedikleri: { $capabilities }.
cli-plugin-sees-nothing = hiçbir şey
cli-plugin-refused = { $path } reddedilirdi ({ $code }): { $reason }
cli-plugin-error-too-large = 2 MiB'tan büyük; bir eklenti en fazla bu kadar olabilir.
cli-plugin-error-format = bir eklenti paketi değil.
cli-plugin-error-newer-format = paket Agora'nın daha yeni bir sürümü için hazırlanmış.
cli-plugin-error-too-many = bir eklentinin sahip olabileceğinden fazla dosya içeriyor (bildirim dışında 64).
cli-plugin-error-path = bir dosyanın adında klasör, yol ya da baştaki nokta var ({ $detail }).
cli-plugin-error-link = paket bir bağlantı ya da dosya baytı olmayan başka bir şey içeriyor ({ $detail }).
cli-plugin-error-type = bir dosya eklentinin kullanamayacağı türde ({ $detail }); yalnızca betik, stil sayfası, görsel, yazı tipi, JSON ve metin.
cli-plugin-error-manifest = bildirim (agora-plugin.json) eksik ya da "{ $detail }" alanında yanlış.
cli-plugin-error-capability = bildirim pencerenin bilmediği bir yetenek istiyor ({ $detail }).
cli-plugin-error-unlisted = bir dosya bildirimde listelenmemiş ({ $detail }).
cli-plugin-error-missing = bildirimin listelediği bir dosya yok ({ $detail }).
cli-plugin-error-hash = bir dosya bildirimin listelediği dosya değil ({ $detail }); karmasını yazmak için --write-hashes ile paketleyin.
cli-plugin-error-newer = eklenti Agora { $detail } ya da daha yenisini istiyor; bu sürüm { $version }.
cli-plugin-error-not-a-file = klasörde düz dosya olmayan bir şey var ({ $detail }): eklenti klasörleri düzdür ve bağlantılar izlenmez.
cli-plugin-error-unreadable = okunamadı ({ $detail }).
cli-plugin-watch-needs-out = --watch için --out DOSYA gerekir.
cli-plugin-out-inside = Paket { $folder } klasörünün kendi içine yazılırdı ({ $out }); klasörün yanına yazın.
cli-plugin-hashes-written = Dosyaların karmaları { $folder }/agora-plugin.json içine yazıldı.
cli-plugin-packed = { $folder }, { $out } içine paketlendi ({ $bytes } bayt).
cli-plugin-watching = { $folder } her değişiklikte yeniden paketleniyor. Durdurmak için Ctrl+C.
cli-bad-arguments = Komut satırı argümanları anlaşılamadı.
cli-mcp-launch-refused = Paylaşılan MCP sunucusu { $item } başlatılmadı: { $reason }.
cli-manual-heading = Kılavuzun konuları bunlar. "agora manual KONU" birini yazdırır; "agora manual --json" konuları JSON olarak listeler.
cli-manual-unknown = Kılavuzda "{ $topic }" adlı bir konu yok. Konuları: { $topics }.
cli-manual-topic-scripting = Ajanları kabuktan başlat, istem gönder, bekle ve durdur; çıkış kodları ve olaylar.
cli-manual-topic-themes = Web penceresi ve terminal istemcisi için bir tema dosyası yaz.
cli-manual-topic-layouts = Bir yerleşim dosyası yaz: bölünmeler, sekme grupları ve bölmeler.
cli-manual-topic-keys = Web penceresi için bir tuş dosyası yaz.
cli-manual-topic-plugins = Bir eklenti yaz: web penceresinde kendi korumalı bölmesinde çalışan bir panel.
cli-manual-topic-mesh = Agora içindeki bir ajanın çağırabileceği mesh araçları.
cli-manual-topic-files = Agora klasöründe her dosyanın yeri ve nasıl okunduğu.
cli-error = Hata { $code }: { $message }
cli-invalid-directory-identity_changed = Bu klasörün proje kimliği, burada son kez ajan başlatıldığından beri değişti. Yine de başlatmak için --accept-project-id-change ekleyin.

doctor-status-ok = tamam
doctor-status-warn = not
doctor-status-fail = SORUN
doctor-line = [{ $status }] { $text }
doctor-version = Agora { $detail }
doctor-pty = Bu sistemde terminaller destekleniyor.
doctor-pty-unsupported = Terminaller { $detail } üzerinde henüz desteklenmiyor.
doctor-local-sockets = Yerel denetim soketleri destekleniyor.
doctor-local-sockets-unsupported = Yerel denetim soketleri { $detail } üzerinde desteklenmiyor; komut satırı burada bir düğüme ulaşamaz.
doctor-sandbox = Bu makinede ajanlar çalışma alanına kısıtlanabilir (düğümün ilkesi: { $detail }).
doctor-sandbox-unsupported = Bu makinede ajanlar kısıtlanamaz (düğümün ilkesi: { $detail }). "off" dışındaki bir değer, ajanların burada başlamayacağı anlamına gelir.
doctor-git = Git kullanılabilir ({ $detail }).
doctor-git-missing = Git bulunamadı. Agora, her ajana ayrı bir çalışma alanı vermek için buna ihtiyaç duyar.
doctor-git-identity-missing = Buradaki ajanların git kimliği yok, bu yüzden commit yapamazlar. "git config --global" ile "user.name" ve "user.email" değerlerini ayarlayın.
doctor-control-token-reachable = Buradaki ajanlar düğümün denetim belirtecini ({ $detail }) okuyabilir, çünkü düğümün klasörü ajanların okuyabildiği veya yazabildiği bir klasörün içinde (örneğin /tmp altında). Ajanlar başlamayacak. Düğümün klasörünü (--data) ilkedeki klasörlerin dışına taşıyın.
doctor-protected-path-reachable = Buradaki ajanlar bu düğümün kendi dosyalarını ({ $detail }) okuyabilir, çünkü bunlar ajanların okuyabildiği veya yazabildiği bir klasörün içinde (/tmp, /var, /opt, /etc ya da /run altında veya ilkedeki bir klasörde) ve bu sistem onları orada kapalı tutamıyor. Ajanlar başlamayacak. Düğümün veri klasörünü (--data) ya da ajan türleri klasörünü (--manifests) bu klasörlerin dışına, örneğin ev klasörünüzün altına taşıyın.
doctor-sockets-profile = Bu makinedeki korumalı alan profili, ajanlara verilmemiş unix soketlerini (ssh-agent, oturum veri yolu, docker, tmux) reddedecek şekilde ayarlıdır. Bir macOS geliştiricisi bunu 17.11.2026'da gerçek bir Mac'te ölçtü: ajanın kendi soketi çalışır, başka bir soket reddedilir ve ad çözücü çalışır. Diğer sistem soketleri (her araçta DNS, https üzerinden git, cargo, npm, Claude Code, Codex) gerçek ajanlarla henüz ölçülmedi; listede adı geçmeyen bir sokete ihtiyaç duyan araç çalışmayabilir, onu `sandbox.sockets` ile ekleyin.
doctor-sockets-abstract = Bu çekirdek dosya olan unix soketlerini kısıtlayamaz: bir ajan hâlâ ssh-agent, oturum veri yolu, docker veya tmux soketine ulaşabilir (yalnızca soyut soketler kapalıdır). Diğer tek katman ortam değişkeni temizliğidir ve soketi arayan bir ajan onu bulabilir.
doctor-sockets-none = Bu makine ({ $detail }) unix soketlerini kısıtlayamaz: tek katman ortam değişkeni temizliğidir ve ssh-agent, oturum veri yolu, docker veya tmux soketini arayan bir ajan onu bulabilir.
doctor-env-scrubbed = Ajanlar bu değişkenleri düğümden devralmaz: { $detail }. Birini geçirmek için düğümün politikasında "sandbox.pass_env" satırına yazın.
doctor-env-passed = Düğümün politikası bu değişkenleri ajanlara geçirir: { $detail }. Her biri bir ajana kullanıcının anahtarlarını veya oturumlarını verebilir.
doctor-data-dir = Veri klasörüne yazılabiliyor: { $detail }
doctor-data-dir-unwritable = Veri klasörüne yazılamıyor: { $detail }
doctor-node = { $detail } adresinde bir düğüm çalışıyor.
doctor-node-not-running = { $detail } adresinde çalışan bir düğüm yok. "agora node run" ile başlatın.
doctor-manifests = { $detail ->
        [one] 1 ajan tanımı yüklendi.
       *[other] { $detail } ajan tanımı yüklendi.
    }
doctor-agent-installed = { $detail } ({ $from ->
        [built-in] yerleşik
        [registry] ACP kayıt defterinden, denenmedi
       *[folder] tanımlar klasöründen
    }): `{ $program }` bulundu{ $by ->
        [agora] , Agora kurdu (sürüm { $version })
        [you] , siz kurmuşsunuz
       *[other] {""}
    }.
doctor-agent-missing = { $detail } ({ $from ->
        [built-in] yerleşik
        [registry] ACP kayıt defterinden, denenmedi
       *[folder] tanımlar klasöründen
    }): bu makine `{ $program }` programını PATH içinde bulamıyor. Programı oraya kurun ya da düğümü `{ $program }` komutunun çalıştığı bir terminalden başlatın.
doctor-agent-broken = { $detail } ({ $from ->
        [built-in] yerleşik
        [registry] ACP kayıt defterinden, denenmedi
       *[folder] tanımlar klasöründen
    }): `{ $program }` orada ama bu hesap onu çalıştıramıyor. Dosyayı çalıştırılabilir yapın ya da yeniden kurun.
doctor-policy-path-dropped = Node politikasındaki "{ $detail }" satırı node'un kullanamayacağı bir yol içeriyor (göreli, ".." ya da ev klasörü olmayan "~"); ajanlar bu yol olmadan başlar. Tam bir yol ya da "~/" ile başlayan bir yol yazın.
doctor-policy-path-missing = Node politikasındaki "{ $detail }" satırı henüz var olmayan bir yol içeriyor; yol oluşana kadar ajanlar onsuz başlar.
doctor-policy-restart = Node, politika dosyası { $detail } dosyasını yalnızca başlarken okur: değiştirdikten sonra node'u yeniden başlatın ("agora node run"). Bir cihaza güvenmek ya da onu kaldırmak yeniden başlatma gerektirmez.
doctor-policy-changed = Politika dosyası { $detail } bu node başladıktan sonra değişti, bu yüzden değişiklik henüz uygulanmıyor. Uygulamak için node'u yeniden başlatın ("agora node run").
doctor-signed-commands = Hub üzerinden bu makineye gelen komutlar, güvendiğin cihazlarından birinin imzasını ister: { $detail }.
doctor-signed-commands-team = Bazı cihazlar belirli projelerle sınırlı; bu yüzden hub üzerinden gelen ve bir şeyi değiştiren her komut, güvenilen bir cihazın imzasını ister: { $detail }.
doctor-signed-commands-none = Hub üzerinden bu makineye gelen komutlar (bir ajanı durdurmak, terminaline yazmak, dosya yazmak) cihazının imzası olmadan çalışır; yani hub'ı ele geçiren biri de bunları gönderebilir. Pencereden bir klasörde ajan başlatmak her zaman imzalıdır. İmza istemek için { $detail } dosyasına şu satırı ekleyip düğümü yeniden başlat: signed_commands = agent.start, agent.stop, agent.prompt, terminal.input, files.write
doctor-devices = { $detail ->
    [one] Bu makinede bir cihaz güvenilir
   *[other] Bu makinede { $detail } cihaz güvenilir
}; bunlardan { $unlimited } tanesi her projede işlem yapabilir.
doctor-devices-none = Bu makinede henüz güvenilen bir cihaz yok, bu yüzden imzalı hiçbir isteği kabul etmez: pencereden gelen onaylar ve başlatmalar burada reddedilir. Tarayıcına ya da terminaline "agora node trust --from-hub" ile güven.
doctor-skip-permissions = { $detail ->
    [allow] Ajanlar, senin onayın ve imzanla, izin sorularını atlayan bir kipte başlatılabilir (politika: skip_permissions = allow).
    [confined_only] Ajanlar izin sorularını yalnızca sınırlandırılmışken atlayabilir (politika: skip_permissions = confined_only).
   *[other] Hiçbir ajan izin sorularını atlayan bir kipte başlatılamaz (politika: skip_permissions = forbid).
}
doctor-policy-problems = Düğümün politika dosyasındaki { $detail } satır anlaşılamadı; bunlar düzelene kadar hiçbir ajan izin sorularını atlayamaz.
doctor-show-secret = Dosya tarayıcısı ve inceleme ekranı, bir projeye bakabilen herkese gizli dosyaları (.env dosyaları, özel anahtarlar, ...) olduğu gibi gösteriyor (politika: files.show_secret = true).
doctor-agent-unsafe = { $detail }: `{ $program }` Agora tarafından kurulmuş, ancak klasörü ya da dosyası güvenli değil (başka bir kullanıcı değiştirebilir); bu yüzden kullanılmıyor: varsa PATH üzerindeki kullanılır. `agora agent install` ile yeniden kurun ya da kurulum klasörlerini yalnızca size ait yapın.
doctor-registry-kinds = ACP kayıt defterinden { $detail ->
        [one] 1 ajan
       *[other] { $detail } ajan
    } biliniyor, Agora'da denenmedi; bunlardan { $ready } tanesi programını bu makinede buluyor. Penceredeki Ayarlar → Ajanlar hepsini listeler.
doctor-runner-folders-left = ACP kayıt defteri ajanlarının başladığı { $detail ->
        [one] 1 klasör
       *[other] { $detail } klasör
    } tamamen silinemedi (düğümün sildiğinden daha derin ya da büyük, ya da kullanmadığı adlar içeriyor); veri klasöründeki `runner` klasöründe `….left` adıyla duruyor ve elle silinebilir.
doctor-registry-off = ACP kayıt defterinin ajanları bu makinede kapalı (politika satırı `agents.registry = off`): makine onları ne sunar ne başlatır. Kendi ajan tanımlarınız etkilenmez.
doctor-registry-denied = ACP kayıt defterinden { $detail ->
        [one] 1 ajan
       *[other] { $detail } ajan
    } bu makinede `agents.registry.deny` politika satırıyla kapalı.
doctor-manifest-older = Manifestler klasöründe { $detail } için daha eski bir tanım var (sürüm { $from }; bu programda { $shipped } yerleşik) ve yerleşik olanın yerine geçiyor; bu yüzden bu türden ajanlar yeniden başlatmadan sonra geri gelmeyebilir. { $file } dosyasını manifestler klasöründen kaldırın: gönderilen tanımlar zaten yerleşik.
doctor-manifest-invalid = { $detail } ajan tanımı okunamadı.
doctor-cert-node = Bu makinenin hub sertifikası { $detail ->
    [one] bir gün daha
   *[other] { $detail } gün daha
} geçerli. Ömrünün üçte biri kaldığında, makine hub'a bağlıyken kendini yeniler.
doctor-cert-node-soon = Bu makinenin hub sertifikası { $detail ->
    [one] bir gün sonra
   *[other] { $detail } gün sonra
} bitiyor. Şimdiye kadar kendini yenilemiş olması gerekirdi: bunu, güncel bir hub'a bağlıyken yapar. Böyle kalırsa hub'ın çalıştığını ve buradan ulaşılabildiğini kontrol et; yoksa süresi bitince bu makine hub'a bağlanamaz ve yeniden katılmalıdır (hub'da "agora hub code" çalıştır, sonra burada yeni kodla "agora node enroll" çalıştır).
doctor-cert-node-expired = Bu makinenin hub sertifikasının süresi doldu, bu yüzden hub'a bağlanamıyor ({ $file }). Yeniden katıl: hub'da "agora hub code" çalıştır, sonra burada yeni kodla "agora node enroll" çalıştır.
doctor-cert-node-unusable = Bu makinenin hub sertifikası sertifika olarak okunamıyor ({ $file }). Dosyanın tam olduğunu ve bu kullanıcının okuyabildiğini kontrol et; dosya bozulduysa makineyi yeniden katıl: hub'da "agora hub code" çalıştır, sonra burada yeni kodla "agora node enroll" çalıştır.
doctor-cert-renew-join-again = Hub, bu makinenin hub sertifikasını yenilemedi ({ $detail }); bu yüzden süresi bitince makine bağlanamaz. Makineyi sahibinin onayıyla yeniden katıl: hub'da "agora hub code" çalıştır (ya da eşleştir), sonra burada yeni kodla "agora node enroll" çalıştır. Ajanlarına ve bekleyen olaylarına dokunulmaz. { $asked ->
    [yes] Hub'a en son { $when } tarihinde soruldu; yaklaşık günde bir yine sorulur, yani hub'ın hatası ise (yedekten geri yüklendi, saati yanlış) bu kendiliğinden geçer.
   *[no] Yaklaşık günde bir yine sorulur, yani hub'ın hatası ise (yedekten geri yüklendi, saati yanlış) bu kendiliğinden geçer.
}
doctor-cert-renew-superseded = Bu makinenin hub sertifikası, hub'ın onun için verdiği en yeni sertifika değil: makinenin sertifikasının başka bir sahibi olabilir. Sahibinden hub'ın denetim görünümünde bu makineye bakmasını iste; nedenini bilmiyorsan makineyi hub'dan kaldır ("agora hub machines") ve yeniden katıl. O zamana kadar makine bu sertifikayla, süresi bitene dek çalışmaya devam eder. { $asked ->
    [yes] Hub'a en son { $when } tarihinde soruldu; yaklaşık günde bir yine sorulur.
   *[no] Yaklaşık günde bir yine sorulur.
}
doctor-cert-renew-rejected = Hub, bu makinenin sertifika yenileme isteğine doğrulanamayan bir sertifikayla karşılık verdi ({ $detail }); bu yüzden kullanılmadı ve kullanımdaki sertifika değişmedi. Bir saat sonra, art arda beş kezden sonra da yeni bir istekle yine sorulur. Tekrarlanırsa bu makinenin kaydolduğu hub'a ulaştığından emin ol.
doctor-cert-renew-clock = Hub'ın bu makine için verdiği yeni sertifika, bu makinenin saatine göre bir günden daha sonra başlıyor ya da çoktan bitmiş; bu yüzden henüz kullanılmıyor. Bu makinenin saati yanlış olabilir: düzelt (ya da otomatik saati aç). İstek saklanır ve her saat yeniden sorulur; saat düzelir düzelmez sertifika alınır. Kullanımdaki sertifika değişmedi.
doctor-cert-renew-hub-old = Bu makinenin bağlı olduğu hub sertifika yenilemeden eski, bu yüzden makinenin sertifikası kendini henüz yenileyemiyor. Hub'ı güncelle (yaklaşık saatte bir yine sorulur); o zamana kadar sertifikası bitmeden bu makineyi yeniden katıl.
doctor-cert-renew-pending = Bu makine için yeni bir sertifika alınıyor; hub henüz yanıtlamadı. Makine elindeki sertifikayla çalışmaya devam eder ve kendisi yine sorar.
doctor-cert-hub = Hub'ın kendi sertifikası { $detail ->
    [one] bir gün daha
   *[other] { $detail } gün daha
} geçerli. Hub, 30 gün kaldığında yeniden başlatmaya gerek kalmadan kendisi yeniler.
doctor-cert-hub-soon = Hub'ın kendi sertifikası { $detail ->
    [one] bir gün sonra
   *[other] { $detail } gün sonra
} bitiyor. Hub, 30 günden az kaldığında sertifikasını kendisi yeniler (her gün bakar); bu yüzden bu dosya olması gerekenden eski. Böyle kalırsa hub'ı yeniden başlat ("agora hub run" ya da servisini yeniden başlat); bitiminden sonra çalışmaya devam eden bir hub yeni bağlantıları reddeder.
doctor-cert-hub-expired = Hub'ın kendi sertifikasının süresi doldu ({ $file }): ona yeni bağlantılar reddediliyor. Yenisini almak için hub'ı yeniden başlat ("agora hub run" ya da servisini yeniden başlat).
doctor-cert-hub-unusable = Hub'ın kendi sertifikası sertifika olarak okunamıyor ({ $file }). Dosyanın tam olduğunu ve hub'ın çalıştığı kullanıcının okuyabildiğini kontrol et; hub'ı yeniden başlatmak ("agora hub run" ya da servisini yeniden başlat) yenisini yazar.
doctor-summary-ok = Agora'nın ihtiyaç duyduğu her şey yerinde.
doctor-summary-problems = Agora'nın ihtiyaç duyduğu bir şey eksik. SORUN olarak işaretlenenleri düzeltin.

cli-hub-listening = Hub { $addr } adresinde çalışıyor. Veri klasörü: { $dir }
cli-hub-names = Hub'ın sertifikasındaki adlar: { $names }.
cli-hub-names-unlisted = Bu bilgisayar ağ adreslerini listelemedi, bu yüzden hub'ın sertifikası onları henüz içermiyor. Hâlâ öyleyse `agora pair open` ne yapılacağını söyler.
cli-hub-notify = Bildirimler { $file } dosyasındaki { $count } hedefe gidiyor.
cli-hub-notify-skipped = { $file } dosyasının { $count } satırı anlaşılamadı ve yok sayılıyor. Her satır `ntfy URL` ya da `webhook URL` olmalı.
cli-hub-web = Web uygulaması: https://{ $addr }/ . { $code } koduyla giriş yapın (10 dakika geçerli; yenisi için `agora hub login`). Hub sertifikasını kendi sertifika yetkilisi (CA) imzaladığı için bu yetkiliye güvenene kadar tarayıcı uyarı verir.
cli-hub-login = Web uygulaması için giriş kodu: { $code } (10 dakika geçerli). Hub adresini tarayıcıda açıp kodu girin.
cli-password-prompt = Yeni ekran parolası (yazarken görünmez):
cli-password-repeat = Bir kez daha yazın:
cli-password-needs-terminal = Ekran parolasını ayarlamak etkileşimli bir terminal ister: parola hiçbir zaman bir borudan ya da dosyadan okunmaz.
cli-password-too-short = Bu çok kısa: en az { $min } karakter kullanın.
cli-password-too-long = Bu çok uzun.
cli-password-mismatch = İkisi aynı değil; hiçbir şey değişmedi.
cli-password-set = Ekran parolası ayarlandı. Başka bir makineden giriş artık bu parolayı ister; bu makinenin kendi tarayıcısına hiç sorulmaz. { $count ->
    [one] Açık bir oturum kapatıldı.
   *[other] Açık { $count } oturum kapatıldı.
  }
cli-password-cleared = Ekran parolası kaldırıldı. Giriş kodu yine tek başına yeterli. { $count ->
    [one] Açık bir oturum kapatıldı.
   *[other] Açık { $count } oturum kapatıldı.
  }
cli-password-on = Ekran parolası ayarlı: başka makinelerden girişler bu parolayı ister.
cli-password-off = Ekran parolası ayarlı değil: giriş kodu tek başına yeterli.
cli-trusted = { $id } cihazı artık bu makinedeki onaylara yanıt verebilir.
cli-trust-failed = Bu cihaz güvenilir olarak kaydedilemedi: { $reason }
cli-trust-revocation-overridden = { $id } cihazı { $revoker } tarafından kaldırılmıştı. O kaldırma yeniden gönderilse de burada güvenilir kalır.
cli-devices-offer = Hub, bu makinenin henüz güvenmediği { $count ->
        [one] bir cihaz
       *[other] { $count } cihaz
    } listeliyor. Her parmak izini, pencerede "Bu cihaz" altında gösterilenle karşılaştırın:
cli-devices-line =   { $name } ({ $id }) · kayıt { $when }
cli-devices-line-user =   { $name } ({ $id }) · { $user } kişisinin · kayıt { $when }
cli-devices-fingerprint =     parmak izi { $fingerprint }
cli-devices-replaces =     Bu makine { $id } cihazına başka bir anahtarla zaten güveniyor: buna güvenmek onu değiştirir (yalnızca --replace ile).
cli-devices-unproven = Hub { $id } cihazını kendi imzası olmadan (anahtarın elde tutulduğunun kanıtı) listeliyor; önerilmiyor.
cli-devices-others = Bu hub'daki başka kişilerin cihazları ({ $count }); yalnızca --include-others ve parmak izleriyle güvenilir:
cli-devices-others-need-flag = Hiçbir cihaza güvenilmedi: bu cihazlar hub sahibinin değil. Birine güvenmek için --yes --include-others --fingerprint PARMAK_İZİ verin.
cli-devices-replace-refused = Güvenilmedi: { $id } cihazına burada başka bir anahtarla zaten güveniliyor. Değiştirmek için --replace verin.
cli-devices-question = Bu cihazlara bu makinede güvenilsin mi? [e/H]
cli-devices-yes = e, evet, y, yes
cli-devices-trusted = { $name } ({ $id }, { $fingerprint }) bu makinede güvenilir oldu.
cli-devices-none = Hub'da henüz kayıtlı cihaz yok. Pencereden oturum açın, sonra burada "agora node trust --from-hub" çalıştırın.
cli-devices-none-new = Hub'da kayıtlı her cihaza bu makinede zaten güveniliyor.
cli-devices-declined = Hiçbir cihaza güvenilmedi.
cli-devices-not-asked = Hiçbir cihaza güvenilmedi: bu etkileşimli bir terminal değil. Bu makinede "agora node trust --from-hub" çalıştırın ya da her cihaz için --yes ile --fingerprint verin.
cli-devices-unmatched = Hub'ın listelediği hiçbir cihazın parmak izi { $fingerprint } değil; güvenilmedi.
cli-devices-not-enrolled = Bu makine bir hub'a kayıtlı değil. Önce "agora node enroll" çalıştırın.
cli-node-in-use = { $dir } veri klasöründe (ya da onun denetim soketinde) zaten başka bir düğüm çalışıyor. Hiçbir şey okunmadı ya da değiştirilmedi. Önce o düğümü durdurun ya da başka bir veri klasörü kullanın.
cli-link-busy = { $dir } bağlantı klasörü, zamanında bitmeyen başka bir Agora komutunun elinde (durdurulmuş ya da yanıt vermiyor). Ondan hiçbir şey okunmadı. O komutu sonlandırıp yeniden deneyin.
cli-link-unreadable = { $dir } bağlantı klasörü okunamadı ({ $reason }). Ondan hiçbir şey okunmadı.
cli-devices-failed = Cihazlar hub'dan alınamadı ({ $reason }).
cli-untrusted = { $id } cihazına artık bu makinede güvenilmiyor.
cli-untrust-unknown = { $id } cihazına bu makinede zaten güvenilmiyordu. Hub onun daha önceki bir onayını getirse de reddedilir.
cli-hub-device-forgotten = { $id } cihazı artık listelenmiyor. Makineler onu reddetmeye devam eder.
cli-hub-device-not-forgotten = { $id } bu hub'da kaldırılmış bir cihaz değil; yalnızca kaldırılmış cihazlar unutulabilir.
cli-hub-devices-none = Henüz kayıtlı cihaz yok. Bir tarayıcı ya da terminal istemcisi oturum açtığında kendini kaydeder.
cli-hub-device-line = { $name } · { $fingerprint } · { $id } · { $user } · kayıt { $created } · son kullanım { $used }{ $state }
cli-device-state-revoked = {" "}· kaldırıldı
cli-device-state-waiting = {" "}· onay bekliyor
cli-hub-backup = Yedek { $path } dosyasına yazıldı.
cli-hub-restored = Veritabanı yedekten { $path } konumuna geri yüklendi. Önceki veritabanı aynı klasörde before-restore adıyla saklanıyor.
cli-hub-failed = Hub komutu başarısız oldu: { $reason }
cli-hub-not-here = Bu makine hub çalıştırmıyor ({ $dir } yok), bu yüzden burada gösterecek bir şey yok. Bu komutu hub'ı çalıştıran makinede çalıştırın ya da pencereyi açın.
cli-hub-fingerprint = Hub sertifika yetkilisinin (CA) parmak izi (SHA-256): { $fingerprint }
cli-hub-user-added = { $id } kullanıcısı { $role } rolüyle eklendi. Giriş kodu: { $code } (10 dakika geçerli). Hub adresini açıp kodu girsin; yenisi için `agora hub user login { $id }` çalıştırın.
cli-hub-user-none = Henüz ekip üyesi yok. `agora hub user add AD --role member` ile ekleyin.
cli-hub-fleet-solo = Ajanlar bu hub'ın her makinesini ve ajanını görür: onu sizden başka kimse kullanmıyor. Kişi eklediğinizde bunları yalnızca `agora hub fleet on` çalıştırırsanız görürler.
cli-hub-fleet-solo-on = Ajanlar bu hub'ın her makinesini ve ajanını görür: onu sizden başka kimse kullanmıyor. Anahtar açık, bu yüzden kişi eklediğinizde onların ajanları da her şeyi görür; erişimleri olmayan projelerdekileri de. `agora hub fleet off` bunu kapatır.
cli-hub-fleet-user-warning = Bu hub'da `agora hub fleet` açık, bu yüzden bu kişinin ajanları her makineyi ve ajanı görür; erişimi olmayan projelerdekileri de. `agora hub fleet off` bunu kapatır.
cli-hub-fleet-on = Ajanlar bu hub'ın her makinesini ve ajanını görür; kişilerinin erişimi olmayan projelerdekileri de. `agora hub fleet off` bunu kapatır.
cli-hub-fleet-off = Bu hub'ı başka kişiler de kullandığı için ajanlar yalnızca kendi projelerinin makinelerini ve ajanlarını görür. `agora hub fleet on` her makineyi ve ajanı görmelerini sağlar.
cli-hub-remote-helpers-solo = Ajanlar bu hub'ın diğer makinelerinden yardımcı başlatmalarını isteyebilir: hub'ı sizden başka kimse kullanmıyor. Her başlatma yine de başlayacağı makinede imzalı onayınızı ya da orada verdiğiniz kalıcı izni gerektirir.
cli-hub-remote-helpers-on = Ajanlar bu hub'ın diğer makinelerinden, başka kişiler de kullansa, yardımcı başlatmalarını isteyebilir. Her başlatma yine de başlayacağı makinede imzalı onay gerektirir. `agora hub remote-helpers off` bunu kapatır.
cli-hub-remote-helpers-off = Bu hub'ı başka kişiler de kullandığı için ajanlar başka makinelerden yardımcı başlatmalarını isteyemez. `agora hub remote-helpers on` buna izin verir.
cli-hub-plugin-added = { $name } { $version } ({ $id }) bu hub için kuruldu. Görmek istedikleri: { $capabilities }. Her kişi pencerede onu gösterip göstermeyeceğini ve neyi görebileceğini kendisi seçer.
cli-hub-plugin-replaced = { $name } { $version } ({ $id }), { $old } sürümünün yerine bu hub için kuruldu. Görmek istedikleri: { $capabilities }. Eski sürüme izin veren herkese, bu sürüm daha fazlasını görmek isterse pencerede yeniden sorulur.
cli-hub-plugin-line = { $id } { $version }  { $name }  görmek istedikleri: { $capabilities }  (kuruldu: { $installed })
cli-hub-plugin-none = Bu hub'da kurulu eklenti yok. `agora hub plugin add FILE` bir eklenti kurar.
cli-hub-plugin-removed = { $id } bu hub'dan, her kişinin ona verdiği izinlerle birlikte kaldırıldı. Pencereler bir sonraki açılışta onu göstermez.
cli-hub-plugin-unknown = Bu hub'da { $id } adlı bir eklenti kurulu değil. `agora hub plugin list` kurulu olanları gösterir.
cli-hub-plugin-full = Bu hub en fazla { $limit } eklenti tutar. Önce birini kaldırın (`agora hub plugin remove ID`).
cli-hub-plugin-unreadable = { $file } okunamadı.
cli-hub-plugin-error-too-large = Bu dosya, bir eklentinin olabileceği en büyük boyut olan 2 MB'tan büyük. Hiçbir şey kurulmadı.
cli-hub-plugin-error-format = Bu dosya bir eklenti paketi değil. Hiçbir şey kurulmadı.
cli-hub-plugin-error-newer-format = Bu eklenti paketi daha yeni bir sürüm için yapılmış. Hiçbir şey kurulmadı.
cli-hub-plugin-error-too-many = Bu paket, bir eklentinin sahip olabileceğinden fazla dosya içeriyor. Hiçbir şey kurulmadı.
cli-hub-plugin-error-path = Bu paket, içinde klasör ya da yol olan bir dosya adı içeriyor ({ $detail }). Hiçbir şey kurulmadı.
cli-hub-plugin-error-link = Bu paket bir bağlantı ya da dosya olmayan başka bir şey içeriyor ({ $detail }). Hiçbir şey kurulmadı.
cli-hub-plugin-error-type = Bu paket, bir eklentinin kullanamayacağı türde bir dosya içeriyor ({ $detail }). Hiçbir şey kurulmadı.
cli-hub-plugin-error-manifest = Paketin açıklaması (agora-plugin.json) eksik ya da "{ $detail }" noktasında hatalı. Hiçbir şey kurulmadı.
cli-hub-plugin-error-capability = Bu eklenti, Agora'nın bilmediği bir şeyi görmek istiyor ({ $detail }). Hiçbir şey kurulmadı.
cli-hub-plugin-error-unlisted = Bu paket, açıklamasında listelenmeyen bir dosya içeriyor ({ $detail }). Hiçbir şey kurulmadı.
cli-hub-plugin-error-missing = Paketin açıklamasında listelenen bir dosya eksik ({ $detail }). Hiçbir şey kurulmadı.
cli-hub-plugin-error-hash = Bu paketteki bir dosya, açıklamasında listelenen dosya değil ({ $detail }). Hiçbir şey kurulmadı.
cli-hub-plugin-error-newer = Bu eklenti Agora { $detail } ya da daha yeni bir sürüm istiyor. Hiçbir şey kurulmadı.
cli-hub-user-line = { $id }  { $name }  { $role }
cli-hub-user-unknown = { $id } adında bir ekip üyesi yok.
cli-hub-user-project =   { $project } projesinde çalışır
cli-hub-user-project-path =   { $project } projesinde çalışır ({ $path })
cli-hub-user-projects-none = Henüz hiçbir makine bu hub'a proje bildirmedi.
cli-hub-user-projects-line = { $name }    { $path }
cli-hub-user-projects-line-host = { $name }    { $path }    ({ $host } makinesi)
cli-hub-user-unknown-machine = { $machine } adında bir makine bu hub'a bağlanmadı. Erişim verebileceğiniz projeleri `agora hub user projects` listeler.
cli-hub-user-unknown-project = Bu hub { $machine } üzerinde { $project } adında bir proje bilmiyor. Erişim verebileceğiniz projeleri `agora hub user projects` listeler.
cli-hub-user-ambiguous-project = { $project } birden fazla projeye uyuyor. Makinesi ve klasörüyle birini seçin, örneğin `agora hub user grant { $id } { $machine } { $path }`:
cli-hub-user-not-granted = { $id } kullanıcısının { $project } projesine erişimi yok.
cli-hub-code = Kayıt kodu: { $code } (10 dakika geçerli). Diğer makinede şunu çalıştırın: agora node enroll --hub wss://ADRES:PORT --ca { $ca } --code { $code } --id AD
cli-enrolled = Bu makine { $id } adıyla kaydedildi. "agora node run" ile başlatın; hub'a kendiliğinden bağlanır. Kayıt yeri: { $dir }.
cli-enroll-refused = Hub bu makineyi kaydetmeyi reddetti ({ $code }). Kod yanlış, süresi dolmuş veya zaten kullanılmış olabilir.
cli-hub-notify-added = { $file } dosyasına eklendi. Kullanmak için hub'ı yeniden başlatın.
cli-hub-notify-none = Henüz bildirim hedefi yok. --ntfy URL ya da --webhook URL ile ekleyin ({ $file }).
cli-hub-notify-bad-url = Bu bir http:// ya da https:// adresi değil.
cli-hub-notify-sent = { $target } hedefine deneme mesajı gönderildi.
cli-hub-notify-refused = { $target }, { $status } durum koduyla yanıt verdi.
cli-hub-notify-unreachable = { $target } hedefine ulaşılamadı: { $reason }
cli-hub-prices-updated = Fiyat tablosu güncellendi: { $count } model.
cli-hub-prices-failed = Fiyat tablosu güncellenemedi: { $reason }
cli-hub-prices-skipped = prices.json içindeki { $count } kayıt anlaşılamadı ve yok sayılıyor. Her kayıtta giriş ve çıkış fiyatı (milyon token başına USD) olmalı.
cli-hub-prices-auto-running = Hub fiyat tablosunu { $url } adresinden kendisi, yaklaşık günde bir alıyor ("agora hub prices auto off" durdurur).
cli-hub-prices-bad-url = Bu bir http:// veya https:// adresi değil.
cli-hub-prices-status = Fiyat kataloğunun sunucusu { $status } durumuyla yanıt verdi, hiçbir şey değişmedi. Yönlendirme izlenmez: son adresi --url ile verin.
cli-hub-prices-not-catalog = Yanıt bir fiyat kataloğu değil (ya da çok büyük), hiçbir şey değişmedi.
cli-hub-prices-shrunk = Katalogda yalnızca { $count } modelin fiyatı var, kullanılan tablonun yarısından az (en az { $floor } gerekir), bu yüzden kullanılmadı.
cli-hub-prices-store-failed = Hub'ın klasörüne yazılamadı: { $reason }
cli-hub-prices-set = Hub artık { $model } modelini sizin fiyatınızla hesaplıyor. Çalışan bir hub bunu birkaç saniye içinde kullanır.
cli-hub-prices-unset = { $model } için fiyatınız kaldırıldı; yeniden kataloğun fiyatı geçerli. Çalışan bir hub bunu birkaç saniye içinde kullanır.
cli-hub-prices-no-override = { $model } için sizin bir fiyatınız yok.
cli-hub-prices-bad-model = Model adı boşluksuz, 1 ile 128 karakter arasında olmalı; örneğin anthropic/claude-sonnet-4-5.
cli-hub-prices-bad-price = Fiyat, milyon token başına ABD doları cinsinden 0 ile 1000000 arasında bir sayıdır.
cli-hub-prices-too-many = Fiyat listenizde zaten en fazla olan { $count } model var. Birini "agora hub prices unset MODEL" ile kaldırın.
cli-hub-prices-overrides-unreadable = { $file } bu komutun değiştirebileceği bir fiyat listesi değil, olduğu gibi bırakıldı. Düzeltin ya da silin, sonra yeniden deneyin.
cli-hub-prices-auto-on = Hub fiyat tablosunu { $url } adresinden kendisi, yaklaşık günde bir alacak; tablosu bir günden eskiyse birkaç dakika içinde başlar. Çalışan bir hub birkaç saniye içinde başlar.
cli-hub-prices-auto-off = Hub fiyat tablosunu artık kendisi almıyor. "agora hub prices update" istediğinizde bir kez alır.
cli-hub-prices-show-bundled = Fiyatlar: bu sürümle gelen tablo ({ $count } model).
cli-hub-prices-show-refreshed = Fiyatlar: { $at } tarihinde alınan tablo ({ $count } model).
cli-hub-prices-show-auto-off = Kendiliğinden alma: kapalı ("agora hub prices auto on" açar).
cli-hub-prices-show-auto-on = Kendiliğinden alma: açık, { $url } adresinden, yaklaşık günde bir.
cli-hub-prices-show-last-ok = Son alma: { $at }, tamam.
cli-hub-prices-show-last-failed = Son alma: { $at }, başarısız ({ $code }); daha sonra yeniden denenir.
cli-hub-prices-show-no-overrides = Sizin fiyatlarınız: yok.
cli-hub-prices-show-override = { $model } için fiyatınız: giriş { $input }, çıkış { $output }, önbellek okuma { $cache_read }, önbellek yazma { $cache_write } (milyon token başına USD).
cli-hub-prices-show-override-bad = { $model } için fiyatınız anlaşılamadı ve yok sayılıyor (giriş ve çıkış gerekir).
cli-hub-themes-on = Tema kataloğu açık: bu hub'a giriş yapan herkes tema düzenleyicide temalarına göz atıp kurabilir. `agora hub themes off` onu yeniden gizler.
cli-hub-themes-off = Tema kataloğu kapalı: pencereler katalog göstermez. `agora hub themes on` bu hub'a giriş yapan herkesin temalarına göz atıp kurmasını sağlar.
cli-hub-themes-never-updated = Henüz bir şey getirilmedi. `agora hub themes update --url URL` kataloğun dizinini getirir (projenin katalog deposu henüz yok, bu yüzden adresini verin).
cli-hub-themes-updated-at = { $count } tema, { $when } tarihinde { $url } adresinden getirildi.
cli-hub-themes-no-url = Henüz katalog adresi yok. Kataloğun index.json adresini `--url` ile verin (projenin katalog deposu henüz yok); sonraki güncellemeler onu yeniden kullanır.
cli-hub-themes-updated = Tema kataloğu güncellendi: { $kept } tema tutuldu, { $refused } reddedildi.
cli-hub-themes-still-off = Katalog kapalı, bu yüzden pencereler onu henüz göstermez. `agora hub themes on` onu açar.
cli-hub-themes-refused = { $id } reddedildi: { $reason }
cli-hub-themes-problem = {"  "}{ $text }
cli-hub-themes-failed = Tema kataloğu güncellenemedi: { $reason } Hiçbir şey değişmedi.
cli-hub-themes-theme-unreachable = { $id } dosyası getirilemedi: { $reason }
cli-hub-themes-index-address = adres https:// değil (düz http:// yalnızca bu makineye izinli).
cli-hub-themes-index-fetch = ona ulaşılamadı.
cli-hub-themes-index-timeout = yanıt vermesi çok uzun sürdü.
cli-hub-themes-index-status = { $status } durumuyla yanıt verdi (yönlendirme izlenmez: son adresi verin).
cli-hub-themes-index-too-large = { $max } KiB'tan büyük.
cli-hub-themes-index-json = JSON değil.
cli-hub-themes-index-format = bir tema kataloğu dizini değil ("agora-theme-catalog": 1 ve bir "themes" listesi gerekir).
cli-hub-themes-index-newer = bu Agora sürümünün okuduğundan daha yeni bir biçimde. Agora'yı güncelleyin.
cli-hub-themes-index-unknown-field = bu sürümün bilmediği bir alanı var: { $field }.
cli-hub-themes-index-too-many = { $max } temadan fazlasını listeliyor.
cli-hub-themes-refused-entry = dizindeki girdisinde geçerli bir { $field } yok.
cli-hub-themes-refused-unknown-field = dizindeki girdisinde bu sürümün bilmediği bir alan var: { $field }.
cli-hub-themes-refused-duplicate = dizindeki başka bir temanın kimliği aynı.
cli-hub-themes-refused-status = dosyası { $status } durumuyla yanıt verdi (yönlendirme izlenmez).
cli-hub-themes-refused-too-large = dosyası { $max } KiB'tan büyük.
cli-hub-themes-refused-total = kataloğun temaları birlikte { $max } KiB'tan büyük olurdu.
cli-hub-themes-refused-hash = dosyası, dizinin verdiği SHA-256 değerine sahip değil.
cli-hub-themes-refused-not-text = dosyası UTF-8 metin değil.
cli-hub-themes-refused-theme = Agora'nın kullanabileceği bir tema değil:
cli-hub-plugins-on = Eklenti kataloğu açık: pencereler eklentilerini kullanılabilir olarak listeler; onları bu makinede `agora hub plugins install ID` ile kurabilirsiniz. `agora hub plugins off` onu yeniden gizler.
cli-hub-plugins-off = Eklenti kataloğu kapalı: pencereler katalog göstermez ve ondan hiçbir şey kurulmaz. `agora hub plugins on` onu açar.
cli-hub-plugins-key-is = Katalog anahtarı: { $fingerprint }. Her dizin, içindeki bir şey kullanılmadan önce ona karşı denetlenir.
cli-hub-plugins-no-key = Katalog anahtarı ayarlanmamış, bu yüzden katalog kullanılamaz. Kataloğun yayımladığı anahtarı `agora hub plugins key set FILE` ile ayarlayın (.pub dosyası; parmak izini projenin yayımladığıyla karşılaştırın).
cli-hub-plugins-key-set = Katalog anahtarı ayarlandı: { $fingerprint }. Onu projenin yayımladığı parmak iziyle karşılaştırın. Başka bir anahtarın yerini aldıysa eskisiyle getirilen katalog unutuldu: `agora hub plugins update` komutunu yeniden çalıştırın.
cli-hub-plugins-key-private = Bu bir özel anahtar. Açık anahtarı (.pub dosyasını) verin; hiçbir şey tutulmadı.
cli-hub-plugins-key-invalid = Bu tek bir ssh-ed25519 açık anahtarı (bir `ssh-ed25519 AAAA…` satırı ya da onu taşıyan bir allowed-signers satırı) değil. Hiçbir şey tutulmadı.
cli-hub-plugins-key-unreadable = { $file } okunamadı.
cli-hub-plugins-never-updated = Henüz bir şey getirilmedi. `agora hub plugins update --url URL` kataloğun dizinini getirir (projenin katalog deposu henüz yok, bu yüzden adresini verin).
cli-hub-plugins-updated-at = { $count } eklenti (dizin seri numarası { $serial }), { $when } tarihinde { $url } adresinden getirildi.
cli-hub-plugins-no-url = Henüz katalog adresi yok. Kataloğun index.json adresini `--url` ile verin (projenin katalog deposu henüz yok); sonraki güncellemeler onu yeniden kullanır.
cli-hub-plugins-updated = Eklenti kataloğu güncellendi: { $kept } eklenti tutuldu, { $refused } reddedildi (dizin seri numarası { $serial }). Hiçbir şey kurulmadı ya da yükseltilmedi.
cli-hub-plugins-newer = Katalogda { $id } için daha yeni bir sürüm var: { $old } → { $new }. `agora hub plugins upgrade { $id }` neyin değiştiğini gösterir ve yükseltmeden önce sorar.
cli-hub-plugins-still-off = Katalog kapalı, bu yüzden pencereler onu listelemez ve ondan hiçbir şey kurulmaz. `agora hub plugins on` onu açar.
cli-hub-plugins-failed = Eklenti kataloğu güncellenemedi: { $reason } Hiçbir şey değiştirilmedi.
cli-hub-plugins-refused = { $id } reddedildi: { $reason }
cli-hub-plugins-refused-field = dizindeki kaydında geçerli bir { $field } yok.
cli-hub-plugins-refused-unknown-field = dizindeki kaydında bu sürümün bilmediği bir alan var: { $field }.
cli-hub-plugins-refused-duplicate = dizindeki başka bir eklentinin kimliği aynı.
cli-hub-plugins-shot-refused = { $id } eklentisinin { $number }. ekran görüntüsü saklanmadı ({ $reason ->
    [fetch] indirilemedi
    [size] boyutu dizinin verdiği boyut değil ya da çok büyük
    [hash] SHA-256 değeri dizinin verdiği değer değil
    [kind] adının söylediği türde bir PNG ya da WebP görseli değil
   *[pixels] 2560 × 1600 pikselden büyük
}); eklenti saklandı.
cli-hub-plugins-index = dizin: { $reason }
cli-hub-plugins-signature-fetch = dizinin imzası (index.json.sig): { $reason }
cli-hub-plugins-fetch-address = kullanılabilir bir adresi yok: https:// olmalı (düz http:// yalnızca bu makineye).
cli-hub-plugins-fetch-fetch = ulaşılamadı.
cli-hub-plugins-fetch-timeout = yanıt vermesi çok uzun sürdü.
cli-hub-plugins-fetch-status = { $status } durum koduyla yanıt verdi (yönlendirme izlenmez: son adresi verin).
cli-hub-plugins-fetch-too-large = { $max } KiB'tan büyük.
cli-hub-plugins-index-json = JSON değil.
cli-hub-plugins-index-format = bir eklenti kataloğu dizini değil ("agora-plugin-catalog": 1, bir "serial" ve bir "plugins" listesi gerekir).
cli-hub-plugins-index-newer = Agora'nın bu sürümünün okuduğundan daha yeni bir biçimde. Agora'yı güncelleyin.
cli-hub-plugins-index-unknown-field = bu sürümün bilmediği bir alan içeriyor: { $field }.
cli-hub-plugins-index-too-many = { $max } eklentiden fazlasını listeliyor.
cli-hub-plugins-signature-malformed = dizinin imzası bu sürümün okuduğu bir SSH imzası değil.
cli-hub-plugins-signature-wrong-key = dizin, bu hub'da ayarlı katalog anahtarından başka bir anahtarla imzalanmış.
cli-hub-plugins-signature-namespace = dizinin imzası eklenti kataloğu için değil, başka bir kullanım için yapılmış.
cli-hub-plugins-signature-invalid = dizinin imzası ona uymuyor: dizin ya da imzası imzalandıktan sonra değiştirilmiş.
cli-hub-plugins-rollback = dizin, bu hub'ın son aldığından (seri { $seen }) daha eski (seri { $offered }); o zamandan beri geri çekilmiş sürümleri adlandırıyor olabilir.
cli-hub-plugins-serial-reused = dizin, bu hub'ın son aldığı dizinin serisini (seri { $serial }) taşıyor ama içeriği başka: imzalı bir dizin yeni bir seri olmadan asla değiştirilmez.
cli-hub-plugins-not-installed = Hiçbir şey kurulmadı: { $reason }
cli-hub-plugins-line = {"  "}{ $text }
cli-hub-plugins-install-asks = { $id } görmek istediği: { $capabilities }
cli-hub-plugins-install-what = { $name } { $version }, yapan { $author }, lisans { $licence }, eklenti kataloğundan; paketi imzalı dizine karşı denetlendi.
cli-hub-plugins-upgrade-asks = { $id } için görmek istedikleri değişiyor:
cli-hub-plugins-caps-added = daha fazlasını görmek istiyor: { $capabilities }
cli-hub-plugins-caps-removed = artık istemiyor: { $capabilities }
cli-hub-plugins-caps-same = yeni bir şey istemiyor
cli-hub-plugins-caps-kept = hâlâ istiyor: { $capabilities }
cli-hub-plugins-upgrade-version = { $name } { $old } → { $new }, yapan { $author }, lisans { $licence }; paketi imzalı dizine karşı denetlendi.
cli-hub-plugins-confirm = Bu hub'a kurulsun mu? [e/H]
cli-hub-plugins-needs-yes = Hiçbir şey kurulmadı: sorulacak bir terminal yok. Yukarıdakileri okuduktan sonra --yes ekleyin.
cli-hub-plugins-declined = Hiçbir şey kurulmadı.
cli-hub-plugins-installed = { $name } { $version } ({ $id }) katalogdan kuruldu. Her kişi pencerede onu gösterip göstermeyeceğini ve neyi görebileceğini kendisi seçer.
cli-hub-plugins-upgraded = { $name } ({ $id }) { $version } sürümüne yükseltildi. Eski sürüme izin veren herkese, bu sürüm daha fazlasını görmek isterse pencerede yeniden sorulur.
cli-hub-plugins-install-off = eklenti kataloğu kapalı. `agora hub plugins on` onu açar.
cli-hub-plugins-install-not-verified = katalog şu anki anahtarla getirilmedi. Önce `agora hub plugins update` çalıştırın.
cli-hub-plugins-install-not-in-catalog = katalogda { $id } adlı bir eklenti yok. `agora hub plugins update` en yeni dizinini getirir.
cli-hub-plugins-install-installed = { $id } { $version } zaten kurulu. `agora hub plugins upgrade { $id }` daha yeni bir sürümü alır.
cli-hub-plugins-install-not-installed = { $id } kurulu değil. `agora hub plugins install { $id }` onu kurar.
cli-hub-plugins-install-not-newer = katalogdaki { $id } ({ $offered }), kurulu olandan ({ $installed }) daha yeni değil.
cli-hub-plugins-install-fetch = paketi: { $reason }
cli-hub-plugins-install-size = paketi imzalı dizinin verdiği boyutta değil.
cli-hub-plugins-install-hash = paketi imzalı dizinin verdiği SHA-256'ya sahip değil: dizin imzalandıktan sonra değiştirilmiş.
cli-hub-plugins-install-mismatch = paketinin açıklaması imzalı dizinden başka bir { $field } söylüyor.
cli-hub-plugins-install-changed = siz yanıtlarken katalog ya da bu hub'da kurulu eklentiler değişti. Şimdi neyi kuracağını görmek için komutu yeniden çalıştırın.
cli-hub-ssh-started = { $file } dosyasındaki { $count } makineye SSH tünelleri açık tutuluyor.
cli-hub-ssh-skipped = { $file } dosyasının { $count } satırı anlaşılamadı ve yok sayılıyor. Her satır `AD HEDEF [PORT]` olmalı.
cli-hub-ssh-none = Henüz SSH makinesi yok. "agora hub ssh add AD HEDEF" ile ekleyin.
cli-hub-ssh-bad = Bu makine eklenemez. Ad yalnızca harf, rakam, ".", "_" veya "-" içerebilir ve daha önce kullanılmamış olmalıdır; hedef "-" ile başlayamaz ve boşluk içeremez.
cli-hub-ssh-added = { $name } makinesi { $file } dosyasına eklendi. Tünelini açmak için hub'ı yeniden başlatın (makinenin sunucu anahtarı known_hosts dosyanızda olmalı). Sonra hub'ın CA sertifikasını ({ $ca }) { $name } makinesine kopyalayıp orada şunu çalıştırın: agora node enroll --hub wss://localhost:{ $port } --ca CA_DOSYASI --code { $code } --id { $name }   (kod 10 dakika geçerli), ardından: agora node run --project YOL
cli-hub-ssh-unknown = { $name } adında bir SSH makinesi yok. Önce "agora hub ssh add AD HEDEF" ile ekleyin.
cli-hub-ssh-install-plan = { $name } ({ $destination }) makinesine düğüm kurulacak: sürüm { $version }, kaynak { $base }. Adımlar: makinedeki mevcut kurulumu denetle, yayınlanan program dosyasını indirip doğrula, tünel üzerinden kaydet, kullanıcı hizmeti olarak çalıştır. Yönetici yetkisi gerekmez. Devam edilsin mi? [e/H]
cli-hub-ssh-install-cancelled = Hiçbir şey değiştirilmedi.
cli-hub-ssh-install-check = 1/4 Makine denetleniyor…
cli-hub-ssh-install-binary = 2/4 Yayınlanan program dosyası indiriliyor ve doğrulanıyor…
cli-hub-ssh-install-enroll = 3/4 Düğüm kaydediliyor (tünel açık olmalı: hub çalışıyor olmalı)…
cli-hub-ssh-install-run = 4/4 Düğüm başlatılıyor…
cli-hub-ssh-install-done = Bitti: { $name } kaydedildi ve düğümü { $service } kullanıcı hizmeti olarak çalışıyor. Orada onay verebilmek için yine de { $name } üzerinde bir cihaza güvenmeniz gerekir ("agora node trust").
cli-hub-ssh-install-manual = { $name } kaydedildi ama kullanıcı hizmetleri için ne systemd ne de launchd var. Düğümü orada kendiniz başlatın, örneğin: { $command }
cli-hub-ssh-install-failed = { $name } üzerinde "{ $step }" adımında durdu: { $detail } Önceden yapılanlar: { $done }. Hiçbir şey geri alınmadı.
cli-update-current = Agora { $version } en yeni sürüm.
cli-update-available = Agora { $version } hazır (şu an { $current } kullanılıyor). Kurmak için "agora update" çalıştırın.
cli-update-local-network = macOS, Agora'nın yerel ağı kullanıp kullanamayacağını yeniden sorabilir: izin verin (ya da Sistem Ayarları > Gizlilik ve Güvenlik > Yerel Ağ'da `agora`yı açın), sonra düğümü bir kez daha yeniden başlatın. Çalışmakta olan düğüm, izin verdikten sonra yeniden başlatılana kadar başka bir makinedeki hub'a ulaşamaz; bağlı olup olmadığını `agora status` söyler.
cli-update-done = Agora { $version } sürümüne güncellendi. Çalışan ajanlar çalışmaya devam eder; yeni sürümü kullanmak için düğümü yeniden başlatın.
cli-update-failed = Güncelleme yapılamadı: { $reason }
cli-update-check-failed = Güncellemeler denetlenemedi. En yeni sürüm bulunamadı.
cli-update-release-not-found = Güncelleme yapılamadı: En yeni sürüm bulunamadı.
cli-update-checksum = İndirilen dosya yayımlanan sağlama toplamıyla uyuşmuyor. Hiçbir şey değişmedi.
cli-update-signature = İndirilen dosyanın imzası, sabitlenmiş yayın anahtarıyla doğrulanamadı. Hiçbir şey değişmedi.
cli-update-needs-key = Yayının imzalı olması gerekir: açık anahtarını Agora klasörüne release-key adıyla kaydedin (ya da --release-key DOSYA verin). Yalnızca sağlama toplamıyla yetinmek için --unsigned kullanın. Hiçbir şey değişmedi.
cli-adapter-failed = Bağdaştırıcı kurulmadı: { $reason }
cli-adapter-no-registry = Kayıt defteri belirtilmedi. --registry URL verin ya da adresini hub klasöründeki registry-url dosyasına yazın. Hiçbir şey değişmedi.
cli-adapter-no-key = Kayıt defterinin kök anahtarı sabitlenmemiş: Agora klasörüne registry-root-key adıyla kaydedin (ya da --root-key DOSYA verin). Hiçbir şey değişmedi.
cli-adapter-exists = Bu adda bir ajan tanımı zaten ajan tanımları klasöründe var ve bir kayıt defterinden kurulmamış. Hiçbir şey değişmedi.
cli-adapter-about = { $name } { $version }, yayımcı { $publisher } (anahtar { $key }), sağlama toplamı { $hash }. Şunu çalıştırır: { $command }
cli-adapter-ask-new = Bu yayımcıdan daha önce bir şey kurmadınız. Kurulsun mu? [e/H]
cli-adapter-ask-known = Kurulsun mu? [e/H]
cli-adapter-declined = Hiçbir şey kurulmadı.
cli-adapter-installed = { $name } { $version } kuruldu. Yalnızca bu türde bir ajan başlatıldığında çalışır.
cli-adapter-removed = { $name } kaldırıldı.
cli-adapter-current = { $name } { $version } kayıt defterindeki en yeni sürüm.
cli-adapter-none = Kurulu bağdaştırıcı yok.
cli-adapter-listed = { $name } { $version } (yayımcı { $publisher })
cli-adapter-listed-revoked = { $name } { $version } (yayımcı { $publisher }, en yeni dizinde iptal edilmiş: kaldırın ya da yeni bir yayımcı bulun)

cli-project-identity = Proje kimliği: { $id } (klasör: { $path }). .agent-project.json dosyasını projenin diğer kopyalarına taşıyın.

cli-panel = Hub'ın paneli bu makinede şu adreste açık: { $url }. Bu adresi tarayıcıda açın; sertifika gerekmez. Giriş için, her tarayıcıda olduğu gibi, hub makinesinde "agora hub login" ile alınan kodu kullanın. Kapatmak için Ctrl+C'ye basın.
cli-panel-none = Bu makine bir hub'a kayıtlı değil, açılacak panel yok. Önce "agora node enroll" ile kaydedin.
cli-panel-link = Bu makinenin hub adresi veya sertifikası okunamıyor. "agora node enroll" ile yeniden kaydedin.
cli-panel-port-taken = Panelin { $port } numaralı portunu başka bir program kullanıyor. O programı kapatıp "agora panel" komutunu yeniden çalıştırın; girişiniz ve cihazınız korunsun diye port değişmez.
cli-panel-no-port = Panel bu makinede bir port açamadı.
cli-panel-port-unsaved = Panelin portu kaydedilemedi; bir sonraki "agora panel" başka bir port kullanabilir ve tarayıcınız bunu yeni bir cihaz sayıp yeniden giriş ister. Bu makinedeki node klasörünün yazılabilir olduğunu kontrol edin.
cli-panel-unreachable = Hub bu makineden yanıt vermiyor, açılacak panel yok. Buradaki ajanlar çalışmaya devam eder.
cli-panel-untrusted = Hub'ın sertifikası bu makinenin kaydolduğu sertifikadan değil, panel açılmadı.
cli-panel-opened = Tarayıcınızda açıldı.

# ADR-0062: yalın `agora` (ön kapı), `agora web`, `agora status`, `agora down`.
frontdoor-welcome = Agora'ya hoş geldiniz. Bu makine ne yapsın?
frontdoor-choice-here = 1) Agora'yı burada başlat: ajanlarınız bu makinede çalışır, onları buradan açarsınız
frontdoor-choice-alone = 2) Yalnızca bu makine, tek başına
frontdoor-choice-join = 3) Başka bir makinede çalışan bir Agora'ya katıl
frontdoor-choose = 1, 2 ya da 3 yazıp Enter'a basın:
frontdoor-lan-question = Diğer makineleriniz bağlanabilsin mi? [e/H]
frontdoor-autostart-question = Oturum açtığınızda Agora başlasın mı? [E/h]
frontdoor-autostart-later = Agora oturum açtığınızda kendiliğinden başlamayacak. İstediğiniz zaman `agora service install` bunu ayarlar.
frontdoor-resume-question = Yeniden başlatmadan sonra ajanlar kendiliğinden geri gelsin mi? [E/h]
frontdoor-resume-off = Ajanlar yeniden başlatmadan sonra kendiliğinden geri gelmeyecek; pencere her birini geri getirmeyi önerir. `agora --resume auto --save` bunu yeniden açar.
frontdoor-resume-saved = Kaydedildi. Bu makinede Agora bir sonraki başladığında geçerli olur: şu an çalışan node, `agora down` yapılana kadar başladığı ayarla devam eder.
frontdoor-get-started = Agora bu makinede henüz kurulmadı. Bir terminalde `agora` çalıştırıp tek bir soruyu yanıtlayın. (Ayarlar { $path } dosyasında tutulur.)
frontdoor-set-up-by-hand = Bu makinede elle başlatılmış bir Agora zaten var ({ $path }). `agora` onu henüz devralmıyor: `agora hub run` ve `agora node run` ile devam edin ya da `agora` yeni bir kurulum yapsın diye AGORA_HOME değişkenini yeni bir klasöre yöneltin.
frontdoor-saved = { $path } dosyasına kaydedildi.
frontdoor-bad-machine-file = { $path } okunamadı. Düzeltin ya da silip ilk soruyu yeniden yanıtlamak için `agora` komutunu tekrar çalıştırın.
frontdoor-not-set-up = Agora bu makinede henüz kurulmadı. Önce `agora` çalıştırın.
frontdoor-no-port = { $port } ve sonrasında boş port bulunamadı.
frontdoor-failed = Agora başlatılamadı: { $reason }
frontdoor-lock-busy = Başka bir süreç { $path } dosyasını 10 saniyeden uzun süredir tutuyor (durdurulmuş ya da takılmış olabilir), bu yüzden Agora çalışanları değiştirmedi. `agora status` ile bulun ya da kapatın ve yeniden deneyin.
frontdoor-lock-unsafe = { $path } (ya da klasörü) bir bağlantı, başka bir dosyanın ikinci adı ya da düz bir dosya veya klasör değil, bu yüzden Agora onu kullanmadı. Kaldırın, Agora yenisini oluşturur.
frontdoor-lock-not-yours = { $path } (ya da klasörü) başka bir kullanıcıya ait, bu yüzden Agora onu kullanmadı. Kaldırın ya da kullanıcınıza geri verin.
frontdoor-role-hub = hub
frontdoor-role-node = node
frontdoor-starting = { $role } başlatılıyor…
frontdoor-in-job = { $role } kendisini başlatan terminalin iş nesnesinden (job) ayrılamadı; bu yüzden o terminal kapanınca o da kapanır. Çalışmaya devam etmesi için `agora` komutunu o işin dışındaki bir terminalden (ya da bir hizmetten) başlatın.
frontdoor-not-ready = { $role } { $seconds } sn içinde başlamadı. Yazdıkları: { $log }
frontdoor-not-linked = Bu makine { $seconds } sn içinde hub'ına bağlanmadı. Yazdıkları: { $log }
frontdoor-running = Agora bu makinede çalışıyor. Bu pencereyi kapatsanız da çalışmaya devam eder; `agora status` durumu gösterir, `agora down` durdurur.
frontdoor-web-failed = Tarayıcıda oturum açılamadı: { $reason }. `agora web` komutunu yeniden çalıştırın; `agora status` Agora'nın çalışıp çalışmadığını gösterir.
frontdoor-web-opened = Agora tarayıcınızda, oturum açılmış olarak açıldı.
frontdoor-web-url = Oturum açmak için bu adresi tarayıcınızda açın (bir kez, 10 dakika geçerli; süresi dolduysa `agora web` komutunu yeniden çalıştırın): { $url }
status-machine = Bu makinenin Agora'sı ({ $path }); arayüz: { $ui }
status-running = { $role } çalışıyor (süreç { $pid }, sürüm { $version })
status-running-supervised = { $role } çalışıyor (süreç { $pid }, sürüm { $version }; gözeticisi süreç { $supervisor })
status-running-other = { $role } çalışıyor (`agora` tarafından başlatılmadı)
status-stopped = { $role } çalışmıyor
status-hub = dinlediği adres { $addr }; veri klasörü { $dir }
status-node-linked = { $hub } adresine { $id } olarak bağlı
status-node-not-linked = bir hub'a bağlı değil
status-node-connecting = { $hub } adresine henüz bağlı değil: düğüm başladı ve bağlanmayı deniyor
status-node-link-unknown = { $hub } adresiyle bağlantı bilinmiyor: bu düğüm bunu bildirmiyor; görmek için düğümü yeniden başlatın
status-node-link-unreachable = { $hub } adresine bağlı değil: bu makine ona ulaşamıyor ({ $reason })
status-node-link-unreachable-macos = { $hub } adresine bağlı değil: macOS ona giden yolu engelliyor ({ $reason }). Sistem Ayarları > Gizlilik ve Güvenlik > Yerel Ağ'da `agora`ya izin verin, sonra düğümü bir kez daha yeniden başlatın.
status-node-link-not-permitted = { $hub } adresine bağlı değil: bu makine bağlantıya izin vermedi ({ $reason })
status-node-link-not-permitted-macos = { $hub } adresine bağlı değil: macOS bağlantıya izin vermedi ({ $reason }). Sistem Ayarları > Gizlilik ve Güvenlik > Yerel Ağ'da `agora`ya izin verin, sonra düğümü bir kez daha yeniden başlatın.
status-node-link-refused = { $hub } adresine bağlı değil: o adreste dinleyen yok ({ $reason })
status-node-link-certificate = { $hub } adresine bağlı değil: hub'ın sertifikasına bu makine güvenmiyor ({ $reason })
status-node-link-hub-refused = { $hub } adresine bağlı değil: hub bu makineyi reddetti (kod { $reason })
status-node-link-closed = { $hub } adresine bağlı değil: hub bağlantıyı kapattı; düğüm yeniden deniyor
status-node-link-other = { $hub } adresine bağlı değil: { $reason }
status-agents = çalışan ajan: { $count }
status-holder-old = sohbet ajanları node yeniden başlayınca sürdürülür, çalışır halde tutulmaz: bu makinenin ajan koruyucusu daha eski ({ $count ->
    [one] 1 ajan tutuyor
   *[other] { $count } ajan tutuyor
}; onlar bitince yenisi başlar)
status-agents-kept =node olmadan çalışmaya devam eden ajan: { $count } (`agora` onları geri alır, `agora down --agents` durdurur)
status-log = günlük: { $log }
down-stopped = { $role } durduruldu (süreç { $pid }).
down-stopped-supervised = { $role } ve gözeticisi durduruldu (süreç { $pid }).
down-failed = { $role } durdurulamadı: { $reason }
down-still-running = durması istendikten 10 saniye sonra hâlâ çalışıyordu
down-lock-busy = Süreç durdu, ama başka bir süreç { $path } dosyasını 10 saniyeden uzun süredir tutuyor (durdurulmuş ya da takılmış olabilir), bu yüzden eski kilit dosyası kaldı: zararsız, bir sonraki başlatma onu değiştirir. O süreci bulun ya da kapatın.
down-nothing = Durdurulacak bir şey yok: `agora` şu anda çalışan hiçbir şey başlatmamış.
down-agents-kept = { $count } ajan çalışmaya devam ediyor. `agora` onları geri alır; `agora down --agents` durdurur.
down-agents-stopped = { $count } ajan durduruldu.
down-holder = ajanların tutucusu
service-description = Oturum açtığınızda Agora { $role } sürecini başlatır (ve çalışır tutar).
service-unsupported = Oturum açınca Agora'yı başlatma bu sistemde henüz yok. Oturum açtıktan sonra `agora` ile başlatın.
service-registering = { $role } oturum açtığınızda başlasın diye "{ $name }" kaydediliyor. Çalıştırdığı komut: { $command }
service-installed = { $role } artık oturum açtığınızda başlıyor ("{ $name }") ve şimdi de başlatıldı.
service-already = { $role } zaten oturum açtığınızda başlıyor ("{ $name }"); hiçbir şey değişmedi. Yeniden kaydetmek için önce `agora service remove` çalıştırın.
service-updated = { $role } hizmeti ("{ $name }") eski ayarlarla kurulmuştu; dosyası güncellendi. Çalışan hizmet, siz yeniden başlatana kadar eski ayarlarla sürer: `systemctl --user restart { $name }`.
service-installed-running = { $role } artık oturum açtığınızda başlıyor ("{ $name }"). Zaten çalışıyor (süreç { $pid }), bu yüzden yeniden başlatılmadı.
service-installed-not-started = { $role } artık oturum açtığınızda başlıyor ("{ $name }"), ama şimdi başlatılamadı: { $reason }. `agora` onu başlatır.
service-bad-path = "{ $path }" oturum açılış kaydına konamıyor: `%` içeriyor (sistem bunu bir değişken sanar) ya da taşıyamadığı karakterler var. Agora'yı ya da klasörünü sade bir yola taşıyın.
supervise-log-link = { $log } günlüğü (ya da klasörü) bir bağlantı, başka bir dosyanın ikinci adı ya da size ait düz bir dosya değil, bu yüzden gözetici ona yazmaz. Onu kaldırın, `agora` yenisini oluşturur.
service-install-failed = "{ $name }" kaydedilemedi: { $reason }
service-removed = "{ $name }" kaldırıldı: { $role } artık oturum açtığınızda başlamıyor.
service-none = { $role } oturum açınca başlayacak şekilde ayarlı değildi.
service-remove-failed = "{ $name }" kaldırılamadı: { $reason }
service-status-installed = { $role } oturum açtığınızda başlıyor ("{ $name }").
service-status-disabled = { $role } oturum açınca başlayacak şekilde ayarlı ("{ $name }"), ama bu, sistem ayarlarında kapatılmış.
service-status-not-installed = { $role } oturum açınca kendiliğinden başlamıyor (`agora service install` bunu ayarlar).
service-status-failed = "{ $name }" okunamadı: { $reason }
service-status-running = şu anda çalışıyor (süreç { $pid })
service-status-stopped = şu anda çalışmıyor
service-unavailable = Oturum açınca Agora'yı başlatma bu makinede kullanılamıyor: Linux'ta systemd'nin kullanıcı hizmetleri (`systemctl`) gerekiyor ve bulunamadı; macOS'ta oturumunuz gerekiyor (ekranda oturum açın; yalnızca SSH yetmez). Oturum açtıktan sonra `agora` ile başlatın.
service-no-user-manager = "{ $name }" için kullanıcınızın hizmet yöneticisine ulaşılamadı: bu oturumda systemd kullanıcı veri yolu yok (örneğin `su` ya da kullanıcı yöneticiniz çalışmıyorken açılmış bir SSH oturumu) ya da macOS'ta oturum yok (ekranda oturum açık değilken bağlanılmış bir SSH oturumu). Bu makinede oturum açıp yeniden deneyin; hiçbir şey değiştirilmedi.
service-in-the-way = { $path } bir bağlantı ya da düz bir dosya değil, bu yüzden Agora onu değiştirmiyor. Saklamanız gereken bir şey değilse silin ve yeniden deneyin.
service-bad-path-control = "{ $path }" oturum açılış kaydına konamıyor: satır sonu ya da başka bir denetim karakteri içeriyor. Agora'yı ya da klasörünü sade bir yola taşıyın.
service-bad-path-relative = "{ $path }" oturum açılış kaydına konamıyor: tam yol değil. Agora'yı tam yoluyla çalıştırın ya da klasörünü tam yolla belirtin.
service-not-ours = "{ $name }" bu makinede zaten kurulu, ama Agora tarafından değil ({ $path }); bu yüzden Agora ona hiç dokunmuyor: değiştirmiyor, durdurmuyor, kaldırmıyor. Agora'nın kurmasını istiyorsanız önce onu kendiniz kaldırın ya da adını değiştirin.
service-root = `agora service` kendi kullanıcınız için bir oturum açılış hizmeti kurar, bu yüzden root olarak (`sudo` ile) çalışmaz. `sudo` olmadan kendiniz çalıştırın; hiçbir şey değiştirilmedi.
service-no-config-folder = Oturum açılış hizmetleriniz için bir klasör yok: ne XDG_CONFIG_HOME ne de HOME tam bir yol gösteriyor. HOME'u ayarlayıp yeniden deneyin; hiçbir şey değiştirilmedi.
service-not-loaded = Yeniden yüklemeden sonra systemd "{ $name }" birimini { $path } konumunda bulamadı; kullanıcı birimlerini başka bir klasörden okuyor (kendi XDG_CONFIG_HOME'u). Hiçbir şey kaydedilmedi.
service-overridden = "{ $name }" yalnızca Agora'nın yazdığını çalıştırmayacaktı: systemd ayrıca { $paths } dosyalarını da okuyor ve bunlar çalışanı değiştirebilir. Hiçbir şey kaydedilmedi. Gerekmiyorsa bu dosyaları kendiniz kaldırın ya da taşıyın ve yeniden deneyin.
service-drop-ins-left = "{ $name }" birimini değiştiren şu dosyaları Agora yazmadı, oldukları gibi bırakıldı: { $paths }
service-bad-path-backslash = "{ $path }" oturum açılış kaydına konamıyor: ters eğik çizgi (\) içeren bir klasör systemd birimine güvenle yazılamaz. Agora'nın klasörünü ters eğik çizgi içermeyen bir yola taşıyın.
service-bad-program = "{ $path }" systemd tarafından başlatılamaz: programın yolu tırnak ya da ters eğik çizgi içeriyor. Agora'yı bunları içermeyen bir yola taşıyın.
service-turned-off-in-settings = "{ $name }", Sistem Ayarları > Genel > Giriş Öğeleri ve Uzantılar ("Arka Planda İzin Ver") bölümünde kapalı ve onu yalnızca siz orada açabilirsiniz. Açın, sonra `agora service install` komutunu yeniden çalıştırın; hiçbir şey değiştirilmedi.
service-was-off = { $role } oturum açınca başlayacak şekilde ayarlıydı ("{ $name }") ama bu kapatılmıştı; yeniden açılıyor.
service-already-overridden = { $role } oturum açtığınızda başlayacak şekilde ayarlı ("{ $name }"), ama systemd ayrıca { $paths } dosyalarını da okuyor ve bunlar çalışanı değiştirebilir. Hiçbir şey değiştirilmedi; gerekmiyorsa bu dosyaları kendiniz kaldırın ya da taşıyın.
service-status-overridden = systemd bunun için ayrıca { $paths } dosyalarını da okuyor; bunlar çalışanı değiştirebilir
service-linger-hint = Bu, Agora'yı oturum açtığınızda başlatır. Oturum açmadan, makine açılınca başlaması için `loginctl enable-linger` komutunu kendiniz çalıştırın; Agora bunu yapmadı.
supervise-not-bound = { $role } başlatılmıyor: bu sistem onun gözeticisine bağlanmasına izin vermedi, bu yüzden `agora down` onu durduramazdı. Elle başlatın (`agora { $role } run`) ya da bunu bildirin.
supervise-stop-request = Gözetici: durdurulması istendi; { $role } de onunla birlikte durduruldu.
supervise-stopped = Gözetici: { $role } durdu (çıkış kodu 0); yeniden başlatılmıyor.
supervise-already-running = Gözetici: { $role } burada zaten çalışıyor (bu gözetici başlatmadı), bu yüzden ikincisi başlatılmıyor.
supervise-lost-lock = Gözetici: { $role } kilidi artık bu gözeticiyi göstermiyor (`agora down` çalıştı ya da başka biri devraldı), bu yüzden { $role } yeniden başlatılmıyor.
supervise-lock-held = Bu Agora klasöründe { $role } ile zaten başka bir süreç (süreç { $pid }) ilgileniyor; bu gözetici kapanıyor.
status-restarting = { $role } çalışmıyor: çöktü ({ $end }) ve gözeticisi (süreç { $pid }) onu birazdan yeniden başlatacak ({ $max } yeniden başlatmadan { $count }.).
status-restarts = son { $window } dakikada { $count } kez yeniden başlatıldı; son çökme: { $end }, { $minutes ->
    [0] bir dakikadan kısa süre önce
   *[other] { $minutes } dk önce
  }
status-gave-up = { $role } { $max } yeniden başlatmadan sonra yine çöktü ve bir daha başlatılmadı ({ $end }, { $minutes ->
    [0] bir dakikadan kısa süre önce
   *[other] { $minutes } dk önce
  }). Nedeni günlüğünde yazıyor; `agora` onu yeniden başlatır.
status-refused = { $role } başlar başlamaz kapandı ({ $end }, { $minutes ->
    [0] bir dakikadan kısa süre önce
   *[other] { $minutes } dk önce
  }), bu yüzden yeniden başlatılmadı. Nedeni günlüğünde yazıyor; `agora` yeniden dener.
supervise-end-code = çıkış kodu { $code }
supervise-end-signal = { $signal } sinyali
supervise-refused = Gözetici: { $role } başladıktan { $seconds } sn sonra kapandı ({ $end }); bu bir çökme sayılamayacak kadar erken, bu yüzden yeniden başlatılmıyor. Nedeni yukarıda.
supervise-restarting = Gözetici: { $role } { $seconds } sn çalıştıktan sonra kapandı ({ $end }); { $wait } sn sonra yeniden başlatılıyor ({ $minutes } dakikada { $max } yeniden başlatmadan { $count }.).
supervise-gave-up = Gözetici: { $role } { $minutes } dakikada { $max } yeniden başlatmadan sonra yine kapandı ({ $end }); vazgeçiliyor. `agora` onu yeniden başlatır.
supervise-handoff-lost = { $role } gözeticisi başlarken durduruldu (`agora down` çalıştı), bu yüzden başlatılmıyor.
cli-node-policy-not-understood = node.policy_not_understood: ilke dosyasında bu makinenin anlamadığı bir satır var ({ $line }); düzeltilene kadar hiçbir ajan sormadan başlatılamaz.
cli-version = Agora { $version }
cli-machines-none = Hub hiçbir makine göstermiyor.
cli-machines-none-old = Eski makine yok: her makine son 24 saatte görüldü.
cli-machine-line = { $id } · { $state ->
    [online] çevrimiçi
    [old] eski, son görülme { $seen }
   *[other] çevrimdışı, son görülme { $seen }
}
cli-machine-cert-soon = { $id } · sertifikası { $date } tarihinde bitiyor{ $days ->
    [0] , bugün
    [1] , bir gün sonra
   *[other] , { $days } gün sonra
}. Bağlıyken kendini yeniler; yenilemezse o makinede "agora doctor" çalıştır ya da yeniden katıl: burada "agora hub code", o makinede "agora node enroll".
cli-machine-cert-expired = { $id } · sertifikası { $date } tarihinde bitti, bu yüzden bağlanamıyor. Yeniden katıl: burada "agora hub code", o makinede "agora node enroll".
cli-machine-unknown = Hub { $id } adında bir makine göstermiyor.
cli-machines-question = Bu makineler unutulsun mu? Her pencereden kalkarlar; yeniden bağlanan geri gelir. [e/H]
cli-machines-question-refuse = Bu makineler unutulsun ve yeniden bağlanırlarsa reddedilsin mi? Her biri yeni bir katılım kodu ister. [e/H]
cli-machines-declined = Hiçbir şey unutulmadı.
cli-machines-not-asked = Hiçbir şey unutulmadı: bu terminal cevap veremiyor. Bir terminalde çalıştırın ya da --yes ekleyin.
cli-machines-hub-unreachable = Çalışan hub'a klasöründen ulaşılamadı ({ $reason }). Makineleri çalışan hub unutur; "agora" ya da "agora hub run" ile başlatın.
cli-machine-forgotten = { $id } unutuldu.
cli-machine-not-forgotten = { $id } unutulamadı ({ $code }).

cli-metrics-token-created = Metrik anahtarı { $name }: { $token }  Yalnızca şimdi gösterilir. Yalnızca `/api/metrics` adresini okuyabilir (hub'ı --metrics ile başlatın); `agora hub metrics-token revoke { $name }` onu sonlandırır.
cli-metrics-token-bad-name = Anahtar adı 1 ile 64 arasında harf, rakam, nokta, tire veya alt çizgiden oluşur.
cli-metrics-token-exists = { $name } adında bir metrik anahtarı zaten var. Önce onu iptal edin ya da başka bir ad seçin.
cli-metrics-token-none = Henüz metrik anahtarı yok. `agora hub metrics-token create AD` ile bir tane oluşturun.
cli-metrics-token-line = { $name } · oluşturuldu { $created } · son kullanım { $used } · bitiş { $expires }{ $state }
cli-metrics-token-never-used = hiç
cli-metrics-token-no-expiry = yok
cli-metrics-token-state-revoked = {" "}· iptal edildi
cli-metrics-token-state-expired = {" "}· süresi doldu
cli-metrics-token-revoked = { $name } metrik anahtarı iptal edildi. Bir sonraki kullanımda reddedilir.
cli-metrics-token-unknown = { $name } adında geçerli bir metrik anahtarı yok.
doctor-credential = { $detail } başlarken bu düğümden { $names } alır (saklanan bir sır, yoksa bu denetimi çalıştıran komutun ortamı; düğümün kendi ortamı farklı olabilir). Yalnızca adlar; bu denetim girişin çalışıp çalışmadığını bilmez.
doctor-credential-none = { $detail } için kimlik bilgisi bulunamadı: düğümün ona verecek bir şeyi yok. "Giriş yapılmadı" derse (kapalı modda çalışan bir ajan Anahtar Zinciri'nde duran girişi göremez), onun için bir jeton üretin ve şunu çalıştırın: agora node secret set { $name }; sonra düğümün politikasına "credentials.{ $detail } = { $name }" satırını ekleyin.
doctor-credential-unlisted = Bu düğümde { $names } için bir değer var, ama politikada { $detail } türünün onu almasına izin veren "credentials.{ $detail }" satırı yok; bu yüzden geçirilmez. Başka ajanlar da onu hiç almaz.
doctor-credential-refused = Politika { $detail } için { $names } adlarını listeliyor, ama türün tanımı bunları bildirmiyor: geçirilmezler.
doctor-holder-old = Bu makinedeki sohbet ajanları node yeniden başlayınca çalışır halde tutulmaz, sürdürülür; çünkü bu makinenin ajan koruyucusu bu programdan daha eski. Tuttuğu ajanlar bitince kendisi de biter ({ $detail ->
    [one] şu an 1 ajan
   *[other] şu an { $detail } ajan
}) ve ardından yenisi başlar.
doctor-credential-malformed =Politikadaki "credentials.{ $detail }" satırı bozuk (virgülle ayrılmış düz adlar, değer yok): { $detail } türüne hiçbir şey geçirilmez.
cli-secret-prompt = { $name } değeri (gösterilmez):
cli-secret-set = { $name } bu makinede saklandı. Değer burada kalır: hub'a asla gönderilmez. Bir ajan türünün onu almasını istiyorsanız düğümün politikasına "credentials.TUR = { $name }" satırını ekleyin ve düğümü yeniden başlatın.
cli-secret-removed = { $name } silindi.
cli-secret-not-found = Burada { $name } adlı bir sır saklı değil.
cli-secret-none = Bu makinede saklı bir sır yok.
cli-secret-name = { $name }
cli-secret-bad-name = Bu ad kullanılamaz: yalnızca harf, rakam ve alt çizgi kullanın, rakamla başlamasın ve sistemin ya da Agora'nın kullandığı bir ad olmasın.
cli-secret-bad-value = Değer en çok 8192 baytlık tek bir satır olmalı, denetim karakteri içermemeli. Hiçbir şey saklanmadı.
cli-secret-failed = Sır burada saklanamadı veya okunamadı. Düğümün veri klasörünün sizin olduğunu ve yazılabildiğini denetleyin.
cli-history-purged = Bu makinenin ajanları için tuttuğu sohbet geçmişi silindi.
cli-history-purge-failed = Sohbet geçmişi silinemedi. Düğümün veri klasörünün size ait ve yazılabilir olduğunu kontrol edin.
cli-secret-name-collides = Adı yalnızca büyük/küçük harfle farklı olan bir sır zaten saklı. O adı aynen kullanın ya da önce onu silin.

# ADR-0065: hub'da `agora pair`, yeni makinede `agora join`.
pair-no-hub = { $dir } klasöründen çalışan bir hub yok. `agora` (ya da `agora hub run`) ile başlatın, sonra bunu hub'ı çalıştıran makinede yeniden çalıştırın.
pair-hub-unreachable = Hub yanıt vermedi ({ $reason }). Çalışıyor mu? `agora` ya da `agora hub run` ile başlatın.
pair-minutes-usage = --minutes 1 ile { $max } arasında bir sayı alır.
pair-opened = Hub'ınız { $minutes } dakika boyunca yeni makinelere açık.
pair-open-next = Yeni makinede `agora join { $address }` çalıştırın. Yeni makine bir sayı gösterir, bu ekran da bir sayı gösterir: yalnızca aynıysa onaylayın.
pair-open-also = Bu adres yeni makineden bu bilgisayara ulaşmıyorsa hub şu adreslerde de yanıt verir: { $addresses }.
pair-open-next-name = Yeni makinede `agora join { $address }` çalıştırın; bu ad oradan bu bilgisayara ulaşıyorsa: hub'ın sertifikası bu adı içerir, ama adın nereye ulaştığına ağınız karar verir. Yeni makine bir sayı gösterir, bu ekran da bir sayı gösterir: yalnızca aynıysa onaylayın.
pair-open-names = Şu adlar hub'ın sertifikasında var ve ağınız onları bu bilgisayara çözümlüyorsa hub'a ulaşabilir: { $names }.
pair-open-next-elsewhere = Yeni makinede `agora join { $address }` çalıştırın; bu adres oradan bu bilgisayara ulaşıyorsa: hub'ın sertifikası bu adresi içerir, ama bu bilgisayarın hiçbir arayüzünde şu an yok, yani eski bir adres ya da ağınız üzerinden buraya ulaşan bir adres olabilir. Yeni makine bir sayı gösterir, bu ekran da bir sayı gösterir: yalnızca aynıysa onaylayın.
pair-open-elsewhere = Hub'ın sertifikası, şu an bu bilgisayarın arayüzlerinde olmayan şu adresleri de içerir: { $addresses }. Bunlar eski bir adres olabilir ya da ağınız üzerinden buraya ulaşabilir.
pair-open-local-only = Bu hub yalnızca bu bilgisayarda dinliyor ({ $listen }), bu yüzden başka bir makine ona ulaşamaz. Diğer makinelerinizin katılabilmesi için bu bilgisayarın ağınızdaki adresinde dinlemesini sağlayın: `agora --listen { $lan }` çalıştırın, sonra `agora down` ve `agora` (`agora hub run` ile başlattığınız bir hub da aynı `--listen` seçeneğini alır), ardından yeniden `agora pair open`. `0.0.0.0:{ $port }` ise bu bilgisayarın her IPv4 arayüzünde dinler, varsa genel (public) bir arayüzde de: ona ulaşabilen herkes bağlanmayı deneyebilir (yeni bir makine yine de sizin onayınıza ve sayıya ihtiyaç duyar).
pair-open-local-only-several = Bu hub yalnızca bu bilgisayarda dinliyor ({ $listen }), bu yüzden başka bir makine ona ulaşamaz. Diğer makinelerinizin katılabilmesi için bu bilgisayarın, onların bulunduğu ağdaki adresinde dinlemesini sağlayın. Bu bilgisayarın birkaç adresi var: { $lans }. O ağdaki adresle `agora --listen ADRES:{ $port }` çalıştırın, sonra `agora down` ve `agora` (`agora hub run` ile başlattığınız bir hub da aynı `--listen` seçeneğini alır), ardından yeniden `agora pair open`. `0.0.0.0:{ $port }` ise bu bilgisayarın her IPv4 arayüzünde dinler, varsa genel (public) bir arayüzde de: ona ulaşabilen herkes bağlanmayı deneyebilir (yeni bir makine yine de sizin onayınıza ve sayıya ihtiyaç duyar).
pair-open-local-only-no-lan = Bu hub yalnızca bu bilgisayarda dinliyor ({ $listen }), bu yüzden başka bir makine ona ulaşamaz. Diğer makinelerinizin katılabilmesi için bu bilgisayarın ağınızdaki adresinde (ADRES) dinlemesini sağlayın: `agora --listen ADRES:{ $port }` çalıştırın, sonra `agora down` ve `agora` (`agora hub run` ile başlattığınız bir hub da aynı `--listen` seçeneğini alır), ardından yeniden `agora pair open`. `0.0.0.0:{ $port }` ise bu bilgisayarın her IPv4 arayüzünde dinler, varsa genel (public) bir arayüzde de: ona ulaşabilen herkes bağlanmayı deneyebilir (yeni bir makine yine de sizin onayınıza ve sayıya ihtiyaç duyar).
pair-open-local-join = Bu bilgisayardaki başka bir Agora şimdi katılabilir: orada `agora join { $address }` çalıştırın. O bir sayı gösterir, bu ekran da bir sayı gösterir: yalnızca aynıysa onaylayın.
pair-open-no-address = Hub'ın sertifikası henüz başka bir makinenin bağlanabileceği bir adres içermiyor. Hub'ı `--name ADRES` ile başlatın (yeni makinenin bağlanacağı adres), sonra `agora pair open` komutunu yeniden çalıştırın.
pair-open-waiting = Bir makine bekleniyor. Ctrl+C ile çıkabilirsiniz: hub, süresi dolana ya da `agora pair close` çalıştırana kadar açık kalır.
pair-status-open = Yeni makinelere açık. { $minutes } dakika sonra kapanır.
pair-status-closed = Yeni makinelere kapalı. Birinin katılması için `agora pair open` çalıştırın.
pair-status-waiting = { $count ->
    [one] Bir makine yanıtınızı bekliyor.
   *[other] { $count } makine yanıtınızı bekliyor.
}
pair-closed-now = Hub'ınız yeni makinelere kapalı.
pair-closed-after = Hub'ınız yeni makinelere yeniden kapalı. Bir tane daha eklemek için `agora pair open` çalıştırın.
pair-reopened-elsewhere = Hub'ınız başka bir yerde yeni makinelere yeniden açıldı ({ $minutes } dakika sonra kapanır), bu yüzden bu ekran onu açık bırakıyor. Kapatmak için `agora pair close` çalıştırın.
pair-status-unknown = Hub'ınızın yeni makinelere hâlâ açık olup olmadığı anlaşılamadı ({ $reason }), bu yüzden buradan hiçbir şey kapatılmadı. Görmek için `agora pair status`, kapatmak için `agora pair close` çalıştırın.
pair-time-up = Verdiğiniz süre doldu. Hub'ınız yeni makinelere kapalı.
pair-needs-terminal = Bir makineyi onaylamak için bir terminalin başında olmanız gerekir: `agora pair open` yanıtınızı yalnızca klavyeden okur, hiçbir zaman bir boru hattından ya da betikten değil. Bir terminal penceresinde çalıştırın. Hiçbir şey açılmadı.
pair-too-fast = Bu tuş, soruyu gördükten çok kısa süre sonra geldi, yanıt sayılmaz. İki sayıya da bakın, sonra yanıtlayın.
pair-input-ended = Buradan yanıt yazılamıyor, bu yüzden hub süresi boyunca açık kalır; isteyen makine bir pencerede sizi bekler, ya da `agora pair close` çalıştırın.
pair-card = Yeni makine "{ $name }" ({ $os }, { $address }) katılmak istiyor. Sayı { $number }.
pair-card-ask = O makinedeki sayı aynı mı? Onaylansın mı? [e/H]
pair-card-ended = "{ $name }" artık istemiyor (vazgeçti, süresi doldu ya da hub'ınız yeni makinelere kapatıldı), yanıtlanacak bir şey kalmadı.
pair-card-approved-elsewhere = Bir pencerede yanıtlandı: "{ $name }" onaylandı ve katılıyor.
pair-card-denied-elsewhere = Bir pencerede yanıtlandı: "{ $name }" reddedildi. O makineye hiçbir şey verilmedi.
pair-card-next = ---- Son istek burada yanıtınız olmadan bitti. Şimdi başka bir makine istiyor: yanıtlamadan önce adını ve sayısını okuyun. ----
pair-next-too-fast = Bu tuş yeni isteği okuyabileceğiniz süreden önce geldi, bu yüzden ondan önce yazılan her şeyle birlikte yok sayıldı. İki saniye bekleyin, yukarıdaki ada ve sayıya bakın, sonra yanıtlayın.
pair-approved = Onaylandı. "{ $name }" katılıyor.
pair-denied-now = Reddedildi. O makineye hiçbir şey verilmedi.
pair-replace-ask = "{ $name }" adlı bir makine zaten katılmış. Yenisiyle değiştirilsin mi? Eskisi erişimini kaybeder. [e/H]
pair-left-waiting = Değiştirilmedi. İstek, süresi dolana kadar hub'da açık kalır.

join-help = Başka bir makinede çalışan bir Agora'ya katılmak için o makinede `agora pair open`, sonra burada `agora join` çalıştırın: hub'ı kendisi bulur. Bulamazsa hub'ın adresini verin: `agora join 192.168.1.20:7443`.
join-looking = Yeni makinelere açık bir hub aranıyor...
join-found-ask = "{ $name }" hub'ı { $address } adresinde bulundu. Parmak izi { $fingerprint }. Bunu hub'da `agora pair open` komutunun gösterdiğiyle karşılaştırın: tam olarak aynı değilse hayır deyin. Aynı mı?  [e/H]
pair-fingerprint = Bu hub'ın parmak izi { $fingerprint }. Bu hub'ı kendiliğinden bulan bir makine bunu gösterir ve kişiden karşılaştırmasını ister.
join-none-found = Yeni makinelere açık hiçbir hub yanıt vermedi. Hub'ı çalıştıran makinede `agora pair open` çalıştırın (bir hub yalnızca açıkken bulunabilir), sonra burada `agora join` komutunu yeniden çalıştırın. Ya da adresini verin: `agora join 192.168.1.20:7443`. Bazı ağlar bu tür aramayı taşımaz; adres her zaman çalışır.
join-found-list-head = Yeni makinelere açık birden çok hub var. Bu parmak izlerini, kastettiğiniz hub'da `agora pair open` komutunun gösterdiğiyle karşılaştırın:
join-found-item = { $n }) "{ $name }", adresi { $address }, parmak izi { $fingerprint }
join-which = Hangisinin parmak izi aynı? Numarasını yazın (1 ile { $count } arası) ya da durmak için Enter'a basın:
join-found-note = Aramak yalnızca bir adres bulmanın yoluydu. Bu makine yalnızca sertifika otoritesinin parmak izi onayladığınızla aynı olan bir hub'a katılır; aşağıdaki numara da onaylamadan önce hub'ın ekranındakiyle aynı olmalı.
join-nothing-chosen = Hiçbir şey seçilmedi. Hiçbir şey kaydedilmedi.
join-connecting = { $address } adresine bağlanılıyor...
join-number = Sayı { $number }. Bu makineyi hub'da onaylayın: hub aynı sayıyı gösterir. Sahibinin onayı bekleniyor...
join-done = Katıldı. Bu makine "{ $id }" adıyla kayıtlı. Hub'ın sertifika yetkilisinin (CA) parmak izi (SHA-256) { $fingerprint }; sahibi `agora hub fingerprint` ile karşılaştırabilir.
join-start = Bu makineyi `agora` ile başlatın; hub'a kendiliğinden bağlanır.
join-already = Bu makine zaten bir hub'a katılmış ({ $dir } klasörüne bakın). Başkasına katılmak için `agora join ADRES --replace` çalıştırın.
join-kept-newer = Bu makine onay beklerken aynı hub'a aynı adla ("{ $id }") yeniden kaydedildi. O daha yeni sertifika korunuyor: bu katılımın aldığı artık geçerli değil. Hiçbir şey değiştirilmedi.
join-kept-unsure = Bu makine onay beklerken aynı hub'a aynı adla ("{ $id }") bu katılımla aynı saniyede yeniden kaydedildi. Hub'ın hangi sertifikayı tuttuğu anlaşılamıyor; bu yüzden orada olan bağlantı korunuyor ve hiçbir şey değiştirilmedi. Bu makine bağlanmazsa katılımı --replace ile yeniden çalıştırın.
join-bad-address = Bu makinenin kullanabileceği bir adres değil. Hub'ın adresini 192.168.1.20:7443 ya da hub.example gibi verin.
join-no-key = Bu makine anahtarını üretemedi.
join-unreachable = { $address } adresindeki hub'a ulaşılamadı ({ $reason }). Çalışıyor mu, adres doğru mu? Hiçbir şey kaydedilmedi.
join-tls13-required = { $address } adresindeki hub yalnızca eski türden güvenli bağlantı sunuyor, bu yüzden bu makine ona katılmaz. Hiçbir şey kaydedilmedi.
join-tls-failed = { $address } adresiyle güvenli bağlantı kurulamadı ({ $reason }). Hiçbir şey kaydedilmedi.
join-timeout-answer = Hub zamanında yanıt vermedi. Hiçbir şey kaydedilmedi.
join-timeout-decision = Kimse bu makineyi zamanında onaylamadı. Hiçbir şey kaydedilmedi. Sahibinden `agora pair open` çalıştırmasını isteyip yeniden deneyin.
join-denied = Sahibi hayır dedi. Hiçbir şey kaydedilmedi.
join-closed = Hub yanıt vermeden konuşmayı bitirdi. Hiçbir şey kaydedilmedi.
join-unexpected = Hub, bu makinenin kabul etmediği bir biçimde yanıt verdi. Hiçbir şey kaydedilmedi.
join-name-hint = Hub'ın sertifikası yazdığınız adresi içermeli. Sahibinden hub'ı `--name { $name }` ile başlatmasını isteyin.
join-certificate-invalid = Hub'ın gönderdiği doğrulanamadı ({ $what }). Hiçbir şey kaydedilmedi.
join-store-failed = Sonuç kaydedilemedi: { $reason }
join-settings-unsaved = Bu makinenin ayarları kaydedilemedi. Kurmak için `agora` çalıştırın.
frontdoor-join-address = Hub'ın adresi (192.168.1.20:7443 gibi; yoksa Enter):

# ADR-0067 rung 3: hub'a bu makinenin dışından ulaşmak (`agora remote`).
remote-password_required = Ekran parolası olmadan dışarıdan erişim açılmaz. Önce `agora password set` ile bir parola belirleyin, sonra bunu yeniden çalıştırın.
remote-password_in_use = Ekran parolası bu makinenin dışından gelen bir erişimi koruyor, bu yüzden kalır. Önce `agora remote off` ile o erişimi kapatın (hub herkese açık bir adreste dinliyorsa yerel bir adreste başlatın), sonra parolayı silin.
remote-tailscale_missing = Bu makinede Tailscale kurulu değil (`tailscale` komutu yok). tailscale.com'dan kurun, `tailscale up` ile oturum açın, sonra bunu yeniden çalıştırın.
remote-tailscale_logged_out = Tailscale kurulu ama bu makine tailnet'inizde oturum açmamış. `tailscale up` çalıştırın, sonra bunu yeniden çalıştırın.
remote-tailscale_timeout = Tailscale zamanında yanıt vermedi. Tailnet'iniz için HTTPS'e izin vermenizi isteyen bir bağlantı yazdıysa onu bir kez açın, sonra bunu yeniden çalıştırın.
remote-tailscale_unreadable = Tailscale'in yanıtı kullanılamadı ({ $reason }). Yukarıda yazılanlar dışında hiçbir şey değiştirilmedi. `tailscale status` komutunun çalıştığını ve tailnet'inizde MagicDNS'in açık olduğunu denetleyin.
remote-tailscale_not_operator = Tailscale `tailscale serve` ayarını yalnızca root'un veya operatörünün değiştirmesine izin veriyor ve bu kullanıcı ikisi de değil. Bir kez `sudo tailscale set --operator={ $user }` çalıştırın, sonra bu komutu yeniden çalıştırın. Yukarıda yazılanlar dışında hiçbir şey değiştirilmedi.
remote-serve_port_in_use = Tailscale zaten { $port } numaralı HTTPS bağlantı noktasında sizin bir şeyinizi sunuyor, Agora ona dokunmaz. `--port N` ile başka bir bağlantı noktası seçin.
remote-names-added = Hub'ın sertifikası artık bu makineyi tailnet'inizde şu adlarla anıyor: { $names }. Çalışan hub bunları birkaç saniye içinde alır; durmuş bir hub başlarken alır.
remote-serve-on = Tailnet'inizdeki bir telefondan veya dizüstünden Agora'yı { $url } adresinde açın (bir kod ve ekran parolasıyla oturum açın).
remote-tailscale-loopback-hint = Hub yalnızca bu makinede dinliyor, bu yüzden diğer tailnet makineleri ona henüz ulaşamaz. Bunu `--serve` ile yeniden çalıştırın veya hub'ı `--listen 0.0.0.0:7443` ile başlatın.
remote-password-reminder = Ekran parolası başka makinelerden gelen her giriş için açık. `agora remote off` bunu yeniden kapatır.
remote-status-listen = Hub { $addr } adresinde dinliyor.
remote-status-not-running = Burada çalışan bir hub bulunamadı ({ $dir } içinde adres yok).
remote-status-this-machine = Bu makine: erişilebilir.
remote-status-lan = Yerel ağ: { $on ->
        [yes] erişilebilir
       *[no] erişilemez
    }.
remote-status-tailnet = Tailscale: { $on ->
        [yes] erişilebilir
       *[no] erişilemez
    }.
remote-status-names = Hub'ın sertifikasında bunun için yazılı adlar: { $names }.
remote-status-serve = Tailscale serve: { $url }
remote-status-public = Herkese açık internet: { $on ->
        [yes] erişilebilir
       *[no] erişilemez
    }.
remote-status-password-set = Ekran parolası: ayarlı.
remote-status-password-not-set = Ekran parolası: ayarlı değil.
remote-status-no-password = Bu makinenin dışından bir erişim açık ve ekran parolası ayarlı değil. Hemen `agora password set` ile bir parola belirleyin veya erişimi `agora remote off` ile kapatın.
remote-public-warning = { $cause ->
        [funnel] Tailscale Funnel açık, yani bu makinenin bir şeyi tüm internetten erişilebilir. Onu yalnızca ekran parolası ve giriş kodu korur. Bilerek açmadıysanız `tailscale funnel reset` ile kapatın.
       *[listen] Hub herkese açık bir adreste dinliyor. Onu yalnızca ekran parolası ve yeni bir makinede denetlediğiniz sertifika korur; düz bir adres başka koruma vermez. Daha güvenli yol Tailscale: `agora remote tailscale --serve`.
    }
remote-off-names = Tailnet adları hub'ın listesinden çıktı; sertifika hub'ın bir sonraki başlangıcında bunları bırakır.
remote-off-serve = { $port } numaralı HTTPS bağlantı noktasındaki Tailscale serve kuralı kaldırıldı. Diğer serve kurallarınıza dokunulmadı.
remote-off-serve-gone = Agora'nın eklediği Tailscale serve kuralı zaten yok.
remote-off-serve-damaged = Agora'nın eklediği Tailscale serve kuralının kaydı okunamadı, bu yüzden kayıt silindi. `tailscale serve status` çalıştırın; kural hâlâ duruyorsa kendiniz kaldırın.
remote-off-serve-foreign = { $port } numaralı HTTPS bağlantı noktasındaki kural artık Agora'nın eklediği değil, bu yüzden olduğu gibi bırakıldı.
remote-off-nothing = Agora bu makinenin dışından hiçbir erişimi açmadı.
frontdoor-listen-saved = Hub bir sonraki başlangıcından itibaren { $addr } adresinde dinleyecek (`agora down`, sonra `agora`).
doctor-remote-password-set = Bu makinenin dışından bir erişim açık ve bir ekran parolası onu koruyor.
doctor-remote-password-missing = Bu makinenin dışından bir erişim açık ve ekran parolası ayarlı değil. Hemen `agora password set` çalıştırın veya `agora remote off`.
doctor-remote-password-missing-tailnet = Hub Tailscale üzerinden erişilebilir ve ekran parolası ayarlı değil. `agora password set` çalıştırın.
doctor-remote-public-listen = Hub herkese açık bir adreste dinliyor: onu yalnızca ekran parolası ve sertifika parmak izi korur. Tailscale'i (`agora remote tailscale --serve`) ve yerel bir adreste dinlemeyi tercih edin.
doctor-remote-tailnet = Tailscale çalışıyor; bu makine tailnet'inizde { $name }.
doctor-remote-tailnet-unreachable = Tailscale buradan erişilemiyor ({ $reason }), bu yüzden tailnet adresi çalışmaz. `tailscale status` komutuna bakın.
doctor-remote-names-missing = Hub'ın sertifikası { $names } adlarını henüz içermiyor. Hub'ı yeniden başlatın veya `agora remote tailscale` az önce çalıştırıldıysa birkaç saniye bekleyin.
remote-password_in_use_listen = Hub herkese açık bir adreste dinliyor ve ekran parolası onu koruyor, bu yüzden kalır. Hub'ı durdurun (`agora down`), yerel bir adreste başlatın (`agora --listen 127.0.0.1:7443`), sonra parolayı silin.
remote-hub_not_running = Agora { $dir } içinde çalışan bir hub'ın adresini bulamadı. Hub'ı `agora` ile başlatın, sonra bunu yeniden çalıştırın.
remote-status-public-funnel = Herkese açık internet: Tailscale Funnel üzerinden erişilebilir.
doctor-remote-funnel = Bu makine için Tailscale Funnel açık, yani bir şey tüm internetten erişilebilir. Onu yalnızca ekran parolası ve giriş kodu korur. Bilerek açmadıysanız `tailscale funnel reset` ile kapatın.

# ADR-0067 rung 4: hub'ın önündeki güvenilen ters vekil veya tünel (`agora remote proxy`).
remote-proxy_header_invalid = "{ $header }" istemci adresini taşıyan başlık olamaz: vekilin her zaman kendisinin koyduğu, sade bir başlık adı kullanın (harf, rakam ve tire); tünel sağlayıcınızın belgelediği başlık gibi. Hiçbir şey değiştirilmedi.
remote-proxy_host_invalid = "{ $host }" hub'ın yanıtlayabileceği bir ad değil: `agora.example.com` gibi sade, herkese açık bir ana makine adı kullanın (adres, joker karakter ve port olmaz; en çok 4). Hiçbir şey değiştirilmedi.
remote-proxy_addr_invalid = "{ $addr }" bir vekilin bağlanabileceği bir adres değil (en çok 4). Bu makinedeki bir vekil için adres gerekmez. Hiçbir şey değiştirilmedi.
remote-proxy-on = Hub artık bu makinedeki bir ters vekile güveniyor: yalnızca { $hosts } adlarını (ve bu makinenin kendi adlarını) yanıtlar ve ziyaretçinin gerçek adresini yalnızca "{ $header }" başlığından alır. Çalışan hub bunu birkaç saniye içinde alır. Vekiliniz 127.0.0.1'e o adresle Host göndererek iletiyor ve adres başlığı eklemiyorsa ziyaretçiler bu makinedeki sahip gibi görünür: herkese açık Host adını ve başlığı ayarlayın, parolanın sorulduğunu dışarıdan (mobil veri) deneyin.
remote-proxy-access-hint = Hub artık bu vekil üzerinden internetten erişilebilir. Şiddetle önerilir: ana makine adının önüne tek oturum açma (SSO) sağlayan bir erişim katmanı koyun (Cloudflare Tunnel için Cloudflare Access); böylece giriş sayfasına yalnızca belirlediğiniz kişiler ulaşır. Ekran parolası ve giriş kodu ikinci kilittir, birincisi değil.
remote-proxy-listen-hint = Hub bu makinenin ötesinde de dinliyor, yani vekil olmadan da erişilebilir. Vekilin tek yol olması için yerel bir adreste başlatın (`agora --listen 127.0.0.1:7443`).
remote-status-proxy = Güvenilen vekil: açık, { $hosts } adlarını yanıtlar, gerçek adres "{ $header }" başlığından. Vekiliniz 127.0.0.1'e o adresle Host göndererek iletiyor ve adres başlığı eklemiyorsa ziyaretçiler bu makinedeki sahip gibi görünür: herkese açık Host adını ve başlığı ayarlayın, parolanın sorulduğunu dışarıdan (mobil veri) deneyin.
remote-proxy-warning = Güvenilen vekil üzerinden internetten erişilebilir. Onu yalnızca vekilin kendi erişim kuralları, ekran parolası ve giriş kodu korur. `agora remote off` ile kapatın. Vekiliniz 127.0.0.1'e o adresle Host göndererek iletiyor ve adres başlığı eklemiyorsa ziyaretçiler bu makinedeki sahip gibi görünür: herkese açık Host adını ve başlığı ayarlayın, parolanın sorulduğunu dışarıdan (mobil veri) deneyin.
remote-status-proxy-damaged = Güvenilen vekili açan dosya bozuk. Çalışan bir hub son sağlam ayarları korur (Host ve Origin denetimleri dahil); şimdi başlayan bir hub hiçbir vekile güvenmez ve Host ile Origin denetlemez. `agora remote off` çalıştırın, sonra `agora remote proxy` komutunu yeniden verin.
remote-off-proxy = Hub artık ters vekile güvenmiyor (çalışan hub bunu birkaç saniye içinde fark eder).
doctor-remote-proxy-on = Hub, güvenilen bir vekil üzerinden internetten erişilebilir ({ $hosts }, adres "{ $header }" başlığından). O ana makine adının önüne tek oturum açma sağlayan bir erişim katmanı koyun; bilerek açmadıysanız `agora remote off` ile kapatın. Vekiliniz 127.0.0.1'e o adresle Host göndererek iletiyor ve adres başlığı eklemiyorsa ziyaretçiler bu makinedeki sahip gibi görünür: herkese açık Host adını ve başlığı ayarlayın, parolanın sorulduğunu dışarıdan (mobil veri) deneyin.
doctor-remote-proxy-listen = Güvenilen bir vekil açık, ama hub bu makinenin ötesinde { $addr } adresinde dinliyor. Vekilin tek yol olması için hub'ı yerel bir adreste başlatın.
doctor-remote-proxy-damaged = Güvenilen vekili açan dosya bozuk. Çalışan bir hub son sağlam ayarları korur (Host ve Origin denetimleri dahil); şimdi başlayan bir hub hiçbir vekile güvenmez ve Host ile Origin denetlemez. `agora remote off` çalıştırın, sonra `agora remote proxy` komutunu yeniden verin.
cli-hub-proxy-on = Güvenilen vekil: açık. Hub { $hosts } adlarını yanıtlar ve onun üzerinden internetten erişilebilir.
hold-failed = Terminal tutucu başlatılamadı: { $message }
cli-agent-install-about =
    { $display } ({ $name }) { $version } sürümü, { $platform } için yalnızca bu makineye kurulmak üzere:
      İndirme:    { $url }
      SHA-256:    { $sha256 }
      İmza:       { $signature }
      Yayımcı:    { $publisher } (anahtar { $key })
      Klasör:     { $dir }
      Sonra:      sürümünü doğrulamak için yeni program "{ $verify }" ile çalıştırılır.
    Program sizin kullanıcınız olarak, dosyalarınızla çalışır. Agora klasörü dışında hiçbir şey değişmez, yönetici olarak hiçbir şey çalışmaz.
cli-agent-install-signature-none = yok (yalnızca sağlama toplamı)
cli-agent-install-signature-by = { $url } adresindeki ayrık imza, "{ $key }" anahtarınızla denetlenir
cli-agent-install-ask-name = Kurmak için ajanın adını ({ $name }) yazın; durdurmak için başka bir şey yazın:
cli-agent-install-ask-go-back = { $installed } kurulu ve bu daha eski bir sürüm. Geri dönmek için sürümü ({ $version }) yazın:
cli-agent-install-done = { $name } { $version } kuruldu. Program şurada: { $launcher }. Agora kabuk profilinizi düzenlemez; sonraki bir sürüm bu klasöre bakana kadar başlattığı ajanlar programı burada bulmaz. Çalışan ajanlara dokunulmadı.
cli-agent-install-current = { $name } { $version } zaten kurulu.
cli-agent-install-no-list = İmzalı listeyi ve kök anahtarını verin: --list DOSYA ve --root-key HEX ya da DOSYA (ya da anahtarı Agora klasörüne agent-list-root-key adıyla kaydedin). Hiçbir şey değişmedi.
cli-agent-install-err-index-invalid = Ajan listesi kabul edilemedi ({ $code }: { $detail }). Hiçbir şey değişmedi.
cli-agent-install-err-list-state = Bu makinenin kabul ettiği en yeni ajan listesinin kaydı ({ $path }) okunamıyor ({ $code }: { $detail }), bu yüzden hiçbir liste kabul edilmiyor: aksi halde eski bir liste geri getirilebilirdi. Bunu beklemiyorsanız önce o dosyayı neyin değiştirdiğini bulun. Baştan başlamak için { $path } dosyasını silin ve kurulumu yayıncının en yeni listesiyle yeniden çalıştırın: o liste ilk liste olarak kabul edilir ve sürümü kaydedilir. Hiçbir şey değişmedi.
cli-agent-install-err-not-approved = Kurulmadı: ajanın adı burada yazılmadı ({ $code }: { $detail }). Hiçbir şey değişmedi.
cli-agent-install-err-download-failed = İndirme yapılamadı ({ $code }: { $detail }). Hiçbir şey değişmedi.
cli-agent-install-err-checksum-mismatch = İndirilen dosya listedeki sağlama toplamıyla uyuşmuyor ({ $code }). Hiçbir şey değişmedi.
cli-agent-install-err-signature-invalid = İndirilenin imzası kabul edilemedi ({ $code }: { $detail }). Üreticinin anahtarını Agora klasörüne agent-keys/AD.pub olarak kaydedin. Hiçbir şey değişmedi.
cli-agent-install-err-disk-full = Disk dolu ({ $code }). Hiçbir şey değişmedi.
cli-agent-install-err-verify-failed = Yeni program beklenen sürümü bildirmedi ({ $code }: { $detail }). Yeni sürüm kaldırıldı; daha önce kurulu sürüm varsa değişmedi.
cli-agent-install-err-timeout = Çok uzun sürdü ({ $code }: { $detail }). Hiçbir şey değişmedi.
cli-agent-install-err-unsupported-platform = Listede bu sistem için indirme yok ({ $code }: { $detail }). Hiçbir şey değişmedi.
cli-agent-install-err-in-progress = Bu makinede başka bir kurulum çalışıyor ({ $code }). Hiçbir şey değişmedi.
cli-agent-install-err-revoked = Listenin yayımcısı { $detail } iptal edilmiş ({ $code }). Hiçbir şey değişmedi.
cli-agent-install-err-downgrade = { $detail } sürümü kurulu ve liste daha eskisini gösteriyor ({ $code }). Eski sürüme gitmek için --go-back ekleyin. Hiçbir şey değişmedi.
cli-agent-install-err-unsupported-format = Bu kayıt henüz burada kurulamaz ({ $code }: { $detail }): arşivler desteklenmiyor. Hiçbir şey değişmedi.
cli-agent-install-err-not-offered = Liste bu ajanı indirme olarak sunmuyor ({ $code }: { $detail }). Hiçbir şey değişmedi.
cli-agent-install-err-tool-missing = Bunun için gereken bir program kurulu değil: { $detail } ({ $code }). Hiçbir şey değişmedi.
cli-agent-install-err-io-failed = Bir dosya yazılamadı ({ $code }: { $detail }). Hiçbir şey değişmedi.
cli-agent-install-err-conflict = Bu ad kullanılamaz ({ $code }: { $detail }): başka bir ajan bu program adına zaten sahip ya da ajanın adı bin. Hiçbir şey değişmedi.

## agora agent explain: yakalanmış bir ekranda hangi ekran kuralının karar verdiği. Kural kimlikleri, bölgeler ve durumlar manifestin kendi sözcükleridir.
cli-explain-header = Ajan türü { $kind } ({ $source ->
        [built-in] yerleşik
        [registry] ACP kayıt defterinden
       *[folder] manifest klasöründen
    }), ekran { $cols }x{ $rows } boyutunda yeniden oynatıldı.
cli-explain-region = Kuralların { $region } bölgesinde okuduğu:
cli-explain-region-empty = (boş)
cli-explain-region-cut = (önceki { $count } satır gösterilmiyor)
cli-explain-rule-matched = EŞLEŞTİ { $id } → { $state } (öncelik { $priority }, { $region })
cli-explain-rule-own-missing = hayır   { $id } → { $state } (öncelik { $priority }, { $region }): kendi metni ekranda yok
cli-explain-rule-all-missing = hayır   { $id } → { $state } (öncelik { $priority }, { $region }): all[{ $index }] ekranda yok
cli-explain-rule-any-missing = hayır   { $id } → { $state } (öncelik { $priority }, { $region }): "any" koşullarının hiçbiri ekranda yok
cli-explain-rule-not-present = hayır   { $id } → { $state } (öncelik { $priority }, { $region }): not[{ $index }] ekranda var
cli-explain-decided = Karar: { $state }, { $id } kuralıyla (en yüksek öncelikten başlayarak ilk eşleşen).
cli-explain-undecided = Karar: yok. Hiçbir kural eşleşmedi; node ajanın önceki durumunu korur.
cli-explain-note = Bunlar yalnızca ekran kurallarıdır. Ajanın kendisinin bildirdiği durum (kancaları, ACP ya da adaptör üzerinden) bunlardan önce gelir.
cli-explain-no-rules = { $kind } türünün ekran kuralı yok, bu yüzden ekranından durum okunmaz ve burada açıklanacak bir şey yok. Ajanın bildirdiği durum ya da node'un kendi verdiği durum (çıkınca bitti gibi) bu raporun dışındadır.
cli-explain-unknown-kind = Burada "{ $kind }" adlı bir ajan türü yok. Çalışan bir node'un sunduğu türleri "agora agent kinds" listeler.
cli-explain-bad-file = Ekran dosyası { $path } okunamadı: { $reason }
cli-explain-file-too-large = Ekran dosyası { $path } { $max } bayttan büyük; yakalanmış bir ekran değil.
cli-explain-manifest-problem = Manifest dosyası { $path } atlandı, node da atlar: { $reason }
cli-explain-bad-size = --size terminal boyutunu SÜTUNxSATIR olarak alır, örneğin 120x40 (her biri 2 ile 1000 arasında).
# ADR-0064: `agora workspaces` ve `agora workspaces clean`.
cli-workspaces-empty = Bu düğümün oluşturduğu ve hâlâ diskte duran bir çalışma alanı yok.
cli-workspaces-summary = { $count } çalışma alanı, toplam { $size }; { $cleanable } tanesi şununla silinebilir: agora workspaces clean
cli-workspaces-ledger-unreadable = Düğümün ajan kaydı okunamadı, bu yüzden hangi çalışma alanlarının kullanımda olduğu bilinemiyor. Hiçbir şey değiştirilmedi.
cli-workspaces-node-unasked = Düğüm çalışıyor ama hangi ajanları çalıştırdığı sorulamadı. Hiçbir şey silinmedi.
cli-workspaces-removed = { $path } çalışma alanı silindi
cli-workspaces-would-remove = { $path } çalışma alanı silinecekti
cli-workspaces-forgotten = { $path } çalışma alanı zaten yoktu; artık listelenmiyor.
cli-workspaces-kept = { $path } çalışma alanı tutuldu: { $reason ->
        [changes] commit edilmemiş değişiklikleri var
        [submodules] içinde açılmış bir alt modül var
        [unmerged] dalında başka hiçbir dalda olmayan ve gönderilmemiş commit'ler var
        [links-moved] Git bağlantıları değiştirilmiş, deposuna güvenilemez
        [moved] artık düğümün onu oluşturduğu yerde değil
        [ignored] Git'in yok saydığı ama gerekebilecek dosyalar var: { $paths }
       *[other] Git onu denetleyemedi
    }
cli-workspaces-clean-summary = { $removed } silindi, { $kept } bırakıldı (her birinin nedeni yukarıda).
cli-workspaces-clean-dry-summary = { $removed } silinecekti, { $kept } bırakılacaktı. Hiçbir şey değiştirilmedi.

cli-hub-task-parent-notices-enabled = { $project } ortak panosunda üst ajanlara görev kabul bildirimi gönderilecek.
cli-hub-task-review-enabled = { $project } projesinin ortak panosunda İnceleme kullanılacak.
