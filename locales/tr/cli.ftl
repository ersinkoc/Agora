# Komut satırı çıktısı. Makine tarafından okunan veriler (kimlikler, durumlar, yollar) olduğu gibi yazılır; yalnızca cümleler burada.

cli-usage =
    Kullanım:
      agora [--ui tui|web|none [--save]] [--resume ask|auto [--save]]
      agora web
      agora status
      agora down
      agora service install|remove|status
      agora node run [--socket YOL] [--manifests DİZİN] [--project YOL]... [--data DİZİN] [--worktrees DİZİN] [--resume auto]
      agora agent start TÜR [--cwd YOL] [--worktree] [--project AD] [--name AD] [--setting KİMLİK=DEĞER]... [--accept-project-id-change] [--socket YOL]
      agora project init [--cwd YOL] [--socket YOL]
      agora agent stop AJAN_KİMLİĞİ [--socket YOL]
      agora agent prompt AJAN_KİMLİĞİ METİN... [--socket YOL]
      agora agent list [--socket YOL]
      agora agent kinds [--socket YOL]
      agora terminal attach AJAN_KİMLİĞİ [--socket YOL]
      agora node stdio [--socket YOL]
      agora mesh stdio --socket YOL
      agora hub run [--listen ADDR] [--hub-dir DIR] [--name HOST]... [--web DIR] [--metrics]
      agora hub code [--hub-dir DIR]
      agora hub metrics-token create NAME [--expires DAYS] | list | revoke NAME [--hub-dir DIR]
      agora hub ssh install AD [--yes] [--unsigned] [--release-url URL] [--hub-dir DIR]
      agora hub login [--hub-dir DİZİN]
      agora password set|clear|status [--hub-dir DİZİN]
      agora hub notify [--ntfy URL | --webhook URL | --test] [--hub-dir DIR]
      agora hub prices update [--url URL] [--hub-dir DIR]
      agora hub ssh [add AD HEDEF [--port N]] [--hub-dir DIR]
      agora hub backup [DOSYA] [--keep N] [--hub-dir DİZİN]
      agora hub restore DOSYA [--hub-dir DİZİN]
      agora hub machines [forget AD | forget --old] [--refuse-return] [--yes] [--hub-dir DİZİN]
      agora hub devices [forget CİHAZ_ID] [--hub-dir DİZİN]
      agora node enroll --hub URL --ca FILE --code CODE --id NAME [--yes --fingerprint PARMAK_İZİ]... [--include-others] [--replace]
      agora panel
      agora node trust CİHAZ_ID AÇIK_ANAHTAR_HEX [--data DİZİN]
      agora node trust --from-hub [--yes --fingerprint PARMAK_İZİ]... [--include-others] [--replace] [--data DİZİN]
      agora node untrust CİHAZ_ID [--data DİZİN]
      agora tui [--hub ADRES:PORT --ca DOSYA --code KOD] [--hub-dir DİZİN]
      agora update [--check] [--version X] [--release-url URL] [--release-key DOSYA] [--unsigned]
      agora adapter install AD [--registry URL] [--root-key DOSYA] [--yes] | update [AD] | list | remove AD
      agora doctor [--data DİZİN] [--manifests DİZİN] [--socket YOL]
cli-node-listening = Düğüm çalışıyor. Denetim soketi: { $socket }
cli-node-events-set-aside = Bu makine { $node_id } olarak bağlanmadan önce ({ $old_id } adıyla) kaydedilen olaylar bir kenara ayrıldı ve hub'a gönderilmeyecek: { $path }
cli-agent-started = { $agent_id } ajanı { $worktree } içinde başlatıldı
cli-agent-stopped = { $agent_id } ajanı durduruldu
cli-agent-prompted = İstem { $agent_id } ajanına gönderildi
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
cli-bad-arguments = Komut satırı argümanları anlaşılamadı.
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
       *[folder] tanımlar klasöründen
    }): `{ $program }` bulundu.
doctor-agent-missing = { $detail } ({ $from ->
        [built-in] yerleşik
       *[folder] tanımlar klasöründen
    }): bu makine `{ $program }` programını PATH içinde bulamıyor. Programı oraya kurun ya da düğümü `{ $program }` komutunun çalıştığı bir terminalden başlatın.
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
doctor-manifest-older = Manifestler klasöründe { $detail } için daha eski bir tanım var (sürüm { $from }; bu programda { $shipped } yerleşik) ve yerleşik olanın yerine geçiyor; bu yüzden bu türden ajanlar yeniden başlatmadan sonra geri gelmeyebilir. { $file } dosyasını manifestler klasöründen kaldırın: gönderilen tanımlar zaten yerleşik.
doctor-manifest-invalid = { $detail } ajan tanımı okunamadı.
doctor-cert-node = Bu makinenin hub sertifikası { $detail ->
    [one] bir gün daha
   *[other] { $detail } gün daha
} geçerli. Henüz hiçbir şey onu yenilemiyor: bitmeden bu makineyi yeniden katıl.
doctor-cert-node-soon = Bu makinenin hub sertifikası { $detail ->
    [one] bir gün sonra
   *[other] { $detail } gün sonra
} bitiyor. Henüz hiçbir şey onu yenilemiyor: bitince bu makine hub'a bağlanamaz. Öncesinde yeniden katıl: hub'da "agora hub code" çalıştır, sonra burada yeni kodla "agora node enroll" çalıştır.
doctor-cert-node-expired = Bu makinenin hub sertifikasının süresi doldu, bu yüzden hub'a bağlanamıyor ({ $file }). Yeniden katıl: hub'da "agora hub code" çalıştır, sonra burada yeni kodla "agora node enroll" çalıştır.
doctor-cert-hub = Hub'ın kendi sertifikası { $detail ->
    [one] bir gün daha
   *[other] { $detail } gün daha
} geçerli. Hub her başladığında yeniden üretilir.
doctor-cert-hub-soon = Hub'ın kendi sertifikası { $detail ->
    [one] bir gün sonra
   *[other] { $detail } gün sonra
} bitiyor. Öncesinde hub'ı yeniden başlat ("agora hub run" ya da servisini yeniden başlat) ki yenisini alsın; bitiminden sonra çalışmaya devam eden bir hub yeni bağlantıları reddeder.
doctor-cert-hub-expired = Hub'ın kendi sertifikasının süresi doldu ({ $file }): ona yeni bağlantılar reddediliyor. Yenisini almak için hub'ı yeniden başlat ("agora hub run" ya da servisini yeniden başlat).
doctor-summary-ok = Agora'nın ihtiyaç duyduğu her şey yerinde.
doctor-summary-problems = Agora'nın ihtiyaç duyduğu bir şey eksik. SORUN olarak işaretlenenleri düzeltin.

cli-hub-listening = Hub { $addr } adresinde çalışıyor. Veri klasörü: { $dir }
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
cli-password-set = Ekran parolası ayarlandı. Başka bir makineden giriş artık bu parolayı ister; bu makinenin kendi tarayıcısına hiç sorulmaz. Açık { $count } oturum kapatıldı.
cli-password-cleared = Ekran parolası kaldırıldı. Giriş kodu yine tek başına yeterli. Açık { $count } oturum kapatıldı.
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
cli-hub-user-line = { $id }  { $name }  { $role }
cli-hub-user-unknown = { $id } adında bir ekip üyesi yok.
cli-hub-user-project =   { $project } projesinde çalışır
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
frontdoor-join-later = Buradan katılmak henüz yok. Agora'nın çalıştığı makinede `agora hub code` çalıştırın; sonra burada `agora node enroll` ve `agora node run` çalıştırın (bkz. `agora --help`).
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
status-log = günlük: { $log }
down-stopped = { $role } durduruldu (süreç { $pid }).
down-stopped-supervised = { $role } ve gözeticisi durduruldu (süreç { $pid }).
down-failed = { $role } durdurulamadı: { $reason }
down-still-running = durması istendikten 10 saniye sonra hâlâ çalışıyordu
down-lock-busy = Süreç durdu, ama başka bir süreç { $path } dosyasını 10 saniyeden uzun süredir tutuyor (durdurulmuş ya da takılmış olabilir), bu yüzden eski kilit dosyası kaldı: zararsız, bir sonraki başlatma onu değiştirir. O süreci bulun ya da kapatın.
down-nothing = Durdurulacak bir şey yok: `agora` şu anda çalışan hiçbir şey başlatmamış.
service-description = Oturum açtığınızda Agora { $role } sürecini başlatır (ve çalışır tutar).
service-unsupported = Oturum açınca Agora'yı başlatma bu sistemde henüz yok. Oturum açtıktan sonra `agora` ile başlatın.
service-registering = { $role } oturum açtığınızda başlasın diye "{ $name }" kaydediliyor. Çalıştırdığı komut: { $command }
service-installed = { $role } artık oturum açtığınızda başlıyor ("{ $name }") ve şimdi de başlatıldı.
service-already = { $role } zaten oturum açtığınızda başlıyor ("{ $name }"); hiçbir şey değişmedi. Yeniden kaydetmek için önce `agora service remove` çalıştırın.
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
}. Öncesinde yeniden katıl (henüz hiçbir şey onu yenilemiyor): burada "agora hub code", o makinede "agora node enroll".
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
