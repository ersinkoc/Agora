# Komut satırı çıktısı. Makine tarafından okunan veriler (kimlikler, durumlar, yollar) olduğu gibi yazılır; yalnızca cümleler burada.

cli-usage =
    Kullanım:
      agora node run [--socket YOL] [--manifests DİZİN] [--project YOL]... [--data DİZİN] [--worktrees DİZİN]
      agora agent start TÜR [--cwd YOL] [--worktree] [--project AD] [--name AD] [--accept-project-id-change] [--socket YOL]
      agora project init [--cwd YOL] [--socket YOL]
      agora agent stop AJAN_KİMLİĞİ [--socket YOL]
      agora agent prompt AJAN_KİMLİĞİ METİN... [--socket YOL]
      agora agent list [--socket YOL]
      agora agent kinds [--socket YOL]
      agora terminal attach AJAN_KİMLİĞİ [--socket YOL]
      agora node stdio [--socket YOL]
      agora mesh stdio --socket YOL
      agora hub run [--listen ADDR] [--hub-dir DIR] [--name HOST]... [--web DIR]
      agora hub code [--hub-dir DIR]
      agora hub ssh install AD [--yes] [--unsigned] [--release-url URL] [--hub-dir DIR]
      agora hub login [--hub-dir DİZİN]
      agora hub notify [--ntfy URL | --webhook URL | --test] [--hub-dir DIR]
      agora hub prices update [--url URL] [--hub-dir DIR]
      agora hub ssh [add AD HEDEF [--port N]] [--hub-dir DIR]
      agora hub backup [DOSYA] [--keep N] [--hub-dir DİZİN]
      agora hub restore DOSYA [--hub-dir DİZİN]
      agora node enroll --hub URL --ca FILE --code CODE --id NAME
      agora node trust CİHAZ_ID AÇIK_ANAHTAR_HEX [--data DİZİN]
      agora tui [--hub ADRES:PORT --ca DOSYA --code KOD] [--hub-dir DİZİN]
      agora update [--check] [--version X] [--release-url URL] [--release-key DOSYA] [--unsigned]
      agora adapter install AD [--registry URL] [--root-key DOSYA] [--yes] | update [AD] | list | remove AD
      agora doctor [--data DİZİN] [--manifests DİZİN] [--socket YOL]
cli-node-listening = Düğüm çalışıyor. Denetim soketi: { $socket }
cli-agent-started = { $agent_id } ajanı { $worktree } içinde başlatıldı
cli-agent-stopped = { $agent_id } ajanı durduruldu
cli-agent-prompted = İstem { $agent_id } ajanına gönderildi
cli-agent-list-empty = Bu düğümde ajan yok.
cli-agent-kinds-empty = Bu düğümde ajan tanımı yok.
cli-terminal-detached = { $agent_id } ajanından ayrıldınız.
cli-attach-needs-terminal = Bağlanmak için etkileşimli bir terminal gerekir.
cli-node-not-running = { $socket } adresinde dinleyen bir düğüm yok. "agora node run" ile başlatın.
cli-unknown-project = Bu düğümde { $project } adında kayıtlı proje yok.
cli-ambiguous-project = Birden fazla proje kayıtlı; --project ile birini seçin.
cli-no-projects = Bu düğümde proje yok. --project YOL ile başlatın.
cli-bad-arguments = Komut satırı argümanları anlaşılamadı.
cli-error = Hata { $code }: { $message }

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
doctor-data-dir = Veri klasörüne yazılabiliyor: { $detail }
doctor-data-dir-unwritable = Veri klasörüne yazılamıyor: { $detail }
doctor-node = { $detail } adresinde bir düğüm çalışıyor.
doctor-node-not-running = { $detail } adresinde çalışan bir düğüm yok. "agora node run" ile başlatın.
doctor-manifests = { $detail ->
        [one] 1 ajan tanımı yüklendi.
       *[other] { $detail } ajan tanımı yüklendi.
    }
doctor-manifests-missing = { $detail } adresinde ajan tanımları klasörü yok.
doctor-agent-installed = { $detail } kurulu.
doctor-agent-missing = { $detail } bu makinede kurulu değil.
doctor-policy-path-dropped = Node politikasındaki "{ $detail }" satırı node'un kullanamayacağı bir yol içeriyor (göreli, ".." ya da ev klasörü olmayan "~"); ajanlar bu yol olmadan başlar. Tam bir yol ya da "~/" ile başlayan bir yol yazın.
doctor-policy-path-missing = Node politikasındaki "{ $detail }" satırı henüz var olmayan bir yol içeriyor; yol oluşana kadar ajanlar onsuz başlar.
doctor-manifest-invalid = { $detail } ajan tanımı okunamadı.
doctor-summary-ok = Agora'nın ihtiyaç duyduğu her şey yerinde.
doctor-summary-problems = Agora'nın ihtiyaç duyduğu bir şey eksik. SORUN olarak işaretlenenleri düzeltin.

cli-hub-listening = Hub { $addr } adresinde çalışıyor. Veri klasörü: { $dir }
cli-hub-notify = Bildirimler { $file } dosyasındaki { $count } hedefe gidiyor.
cli-hub-notify-skipped = { $file } dosyasının { $count } satırı anlaşılamadı ve yok sayılıyor. Her satır `ntfy URL` ya da `webhook URL` olmalı.
cli-hub-web = Web uygulaması: https://{ $addr }/ . { $code } koduyla giriş yapın (10 dakika geçerli; yenisi için `agora hub login`). Hub sertifikasını kendi sertifika yetkilisi (CA) imzaladığı için bu yetkiliye güvenene kadar tarayıcı uyarı verir.
cli-hub-login = Web uygulaması için giriş kodu: { $code } (10 dakika geçerli). Hub adresini tarayıcıda açıp kodu girin.
cli-trusted = { $id } cihazı artık bu makinedeki onaylara yanıt verebilir. Uygulanması için bu makinedeki düğümü yeniden başlatın.
cli-trust-failed = Bu cihaz güvenilir olarak kaydedilemedi: { $reason }
cli-hub-backup = Yedek { $path } dosyasına yazıldı.
cli-hub-restored = Veritabanı yedekten { $path } konumuna geri yüklendi. Önceki veritabanı aynı klasörde before-restore adıyla saklanıyor.
cli-hub-failed = Hub komutu başarısız oldu: { $reason }
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
