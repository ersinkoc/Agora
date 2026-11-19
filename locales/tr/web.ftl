# Web istemcisi metinleri. Arayüz sade sözcükler kullanır, iç terimler kullanmaz.

app-name = Agora
home-title = Ana sayfa
gallery-title = Bileşen galerisi
attention-title = Sizi bekleyenler
attention-empty = Her şey sakin. Sizi bekleyen yok.
fleet-title = Ajanlarınız
fleet-empty = Henüz ajan yok. Bir ajan başlatın, burada görünsün.

state-working = Çalışıyor
state-needs-you = Sizi bekliyor
state-finished = Bitti
state-idle = Boşta
state-unknown = Bilinmiyor
state-failed = Hatayla bitti

agent-on-machine = { $machine } üzerinde
agent-workspace = Çalışma alanı: { $name }
machines-online =
    { $count ->
        [one] { $count } makine çevrimiçi
       *[other] { $count } makine çevrimiçi
    }
link-connecting = Hub'a bağlanılıyor…
link-offline = Hub'a ulaşılamıyor. Yeniden deneniyor…
link-retry = Şimdi dene
action-open = Aç
action-start-agent = Ajan başlat
theme-dark = Koyu
theme-light = Açık

palette-title = Komut paleti
palette-placeholder = Bir ajana git veya bir işlem yap…
palette-empty = Eşleşen yok.
palette-open-agent = Ajanı aç: { $name }
palette-next-attention = Beni bekleyen sıradaki
palette-go-home = Ana sayfaya git
palette-hint = Aramak için Ctrl+K veya ⌘K, sizi bekleyen sıradaki ajan için N tuşuna basın.

focus-back = Ana sayfaya dön
focus-terminal = Terminal
focus-terminal-placeholder = Bu pencere makinelerinize bağlandığında canlı terminal burada görünür.
focus-why = Bu neden gösteriliyor?
focus-why-agent = Ajan bunu kendisi bildirdi.
focus-why-screen = Ajanın ekranında görünenden okundu.
focus-not-found = Bu ajan artık burada değil.

approvals-title = Yanıtınızı bekleyenler
approval-title = { $name } izin istiyor
approval-allow = İzin ver
approval-deny = Reddet
approval-not-trusted = Bu cihaza o makine henüz güvenmiyor. "Bu cihaz" sayfasını açın ve gösterdiği komutu makinede çalıştırın.
approval-failed = Yanıt iletilemedi. Yeniden deneyin.
approval-forbidden = Bu cihazdan bu proje için yanıt veremezsiniz: orada başka projelerle sınırlı. Bu projede çalışan birine sorun.
start-approval-title = { $machine } üzerindeki { $name }, bir { $kind } ajanı başlatmak istiyor
start-approval-title-nomachine = { $name }, bir { $kind } ajanı başlatmak istiyor
approval-someone = Bir ajan
start-approval-helper = İsteyen ajan kendisi de bir yardımcı
start-approval-folder = Klasör
start-approval-worktree = kendi çalışma ağacında
start-approval-outside = Bu klasör, isteyen ajanın kendi klasörünün dışında.
start-approval-prompt = İlk istemi
start-approval-no-prompt = İstem yok: başlar ve bekler.
start-approval-scope = Neye izin verilsin
start-approval-once = Yalnızca bu
start-approval-no-grant = Yalnızca bu başlatmaya izin verilebilir: bu tür kendisine verilen her programı çalıştırır, bu yüzden hiçbir zaman sormadan başlatılmaz.
start-approval-settings-once = Yalnızca bu başlatmaya izin verilebilir: kendi modeli veya eforuyla başlatma her zaman sorulur.
start-approval-more = { $name } çalıştığı sürece bu klasörde, yeniden sormadan en fazla { $count } { $kind } ajanı
start-approval-nested = Yardımcıları da yardımcı başlatmak isteyebilir (her birini siz yanıtlarsınız)
helper-started-by = { $name } tarafından başlatıldı
helper-grants = Yardımcı izinleri
helper-grant-line = Sormadan hâlâ { $count } { $kind } başlatabilir, klasör:
helper-grant-revoke = Geri al
helper-grants-failed = Yardımcı izinleri okunamadı ya da değiştirilemedi.
device-title = Bu cihaz
device-intro = Makineler yalnızca güvendikleri bir cihazın yanıtını kabul eder. Bu tarayıcıyı şununla tanırlar:
device-name = Cihaz adı
device-key = Cihaz anahtarı
device-link = Bu cihaz
device-fingerprint = Parmak izi
device-trusted-here = bu tarayıcıya güveniyor
device-not-trusted-here = bu tarayıcıya henüz güvenmiyor
device-add-title = Bu tarayıcıyı ekleyin: başka bir cihazınızda onaylayın
device-add-intro = Makinelerinizden biri başka bir cihazınıza (bir tarayıcı ya da terminal istemcisi) zaten güveniyorsa, o cihaz bunu onaylayabilir. Bu tarayıcının parmak izini, { $fingerprint }, içeren bir kart görür: aynı olduğunu denetleyin.
device-ask = Başka bir cihazdan onay iste
device-asked = İstendi. Makinelerinizin güvendiği bir cihazda pencereyi açın: bu tarayıcı için bir kart gösterir. Parmak izi { $fingerprint } olmalı.
device-ask-failed = İstenemedi. Bağlantıyı denetleyip yeniden deneyin.
device-command-intro = Ya da makinede şunu çalıştırın (`agora` orada bulunamazsa komutu agora programının klasöründe çalıştırın: ./agora, Windows'ta .\agora.exe):
device-others = Diğer cihazlarınız
device-removed = kaldırıldı
device-waiting = onayınızı bekliyor
device-trusted-on = güvenilen makineler: { $machines }
device-trusted-nowhere = hiçbir makinede güvenilmiyor
device-last-used = son kullanım { $when }
device-remove = Kaldır
device-remove-failed = Kaldırılamadı.
device-request-title = { $name } güvenilir olmak istiyor
device-request-compare = O cihazın "Bu cihaz" altında gösterdiği parmak iziyle karşılaştırın. Yalnızca aynıysa onaylayın.
device-request-machines = Şu makinelerde
device-request-all = Hepsinde ve bu cihaza sonradan güvenecek her makinede.
device-request-some = Yalnızca işaretlilerde.
device-request-failed = Onay gönderilemedi. Yeniden deneyin.
device-request-later = Şimdi değil
device-request-approve = Onayla
# ADR-0065: katılmak isteyen bir makine ya da başka bir makinede oturum açmak isteyen bir tarayıcı, sahibinin yanıtını bekler.
pair-machine-title = Yeni bir makine katılmak istiyor
pair-browser-title = Bir tarayıcı oturum açmak istiyor
pair-name = Ad
pair-system = Sistem
pair-address = Adres
pair-number = Numara
pair-failed = Yanıtınız gönderilemedi. Yeniden deneyin.
pair-deny = Reddet
pair-approve = Onayla
pair-machine-compare = Yeni makinedeki numarayla karşılaştırın. Yalnızca aynıysa onaylayın.
pair-browser-compare = O tarayıcıdaki numarayla karşılaştırın. Yalnızca aynıysa onaylayın.
pair-expires-in = { $time } içinde sona erer
pair-ran-out = Bu isteğin süresi doldu. Yeni makine yeniden isteyebilir.
pair-offline = Şu anda bağlı değil. Bağlantı geri gelince yanıtlayabilirsiniz.
pair-confirm = İki numara tamamen aynı mı?
pair-confirm-yes = Evet, numaralar aynı
pair-back = Geri dön
pair-hide = Gizle
pair-replace = Var olan makinenin yerine geç
pair-replace-why = "{ $name }" adlı bir makine zaten katılmış. Yerine geçerseniz eskisi erişimini kaybeder.
pair-sent = Yanıt gönderildi.
pair-row-where = katılmak istiyor
pair-row-what = Onaylamadan önce numarasını makinenin ekranındakiyle karşılaştırın.
pair-review-for = { $name } isteğini incele
pair-section = { $count } makine katılmak istiyor
pair-recent = Son katılma istekleri
pair-result-approved = { $name } onaylandı, katılıyor.
pair-result-denied = { $name } reddedildi.
pair-result-expired = { $name } isteğinin süresi doldu.
pair-result-closed = { $name } isteği iptal edildi.
pair-window-label = Yeni makineler
pair-window-allow = Yeni bir makinenin katılmasına izin ver
pair-window-help = Ya da yeni bir makine kodsuz katılmak istesin: burada izin verin, sonra o makinede agora join çalıştırın. 10 dakika açık kalır.
pair-window-open = Yeni makinelere açık. { $time } sonra kapanır.
pair-window-open-help = Yeni makinede bu bilgisayarın adresiyle agora join çalıştırın. İki ekranda da bir numara görünür: yalnızca aynıysa onaylayın.
pair-window-close = Kapat
pair-window-failed = Olmadı. Yeniden deneyin.
os-windows = Windows
os-macos = macOS
os-linux = Linux
signin-waiting = Sahibinin bu tarayıcıyı onaylaması bekleniyor…
signin-waiting-hint = Sahibi bu numarayı, zaten oturum açtığı bir penceredeki kartta görür ve yalnızca eşleşiyorsa onaylar.
signin-denied = Sahibi bu tarayıcıyı onaylamadı.
signin-expired = Bu tarayıcıyı zamanında onaylayan olmadı.
signin-ask-again = Yeniden iste
signin-use-code = Bunun yerine kod kullan
trust-ask = Başka bir cihazdan onay iste
trust-asked = İstendi. Bu tarayıcıyı o makinenin güvendiği bir cihazda onaylayın, sonra yeniden başlatın.

signin-title = Giriş yap
signin-intro = Hub'ınızdan bir kod isteyin: hub'ı çalıştıran makinede "agora hub login" komutunu çalıştırıp kodu buraya yazın.
signin-code = Kod
signin-action = Giriş yap
signin-refused = Bu kod kabul edilmedi. Yanlış, süresi dolmuş veya daha önce kullanılmış olabilir.
signin-too-many = Buradan çok fazla yanlış deneme yapıldı. Bir süre bekleyip yeniden deneyin.
signin-failed = Hub'ınıza ulaşılamadı. Yeniden deneyin.
signin-password = Ekran parolası
signin-password-hint = Başka bir makineden giriş yaptığınız için bu hub ekran parolasını da istiyor.
signin-refused-password = Giriş yapılamadı. Kod yanlış, süresi dolmuş veya kullanılmış olabilir; ya da parola yanlış olabilir.
lock-now = Kilitle
lock-now-hint = Bu pencereden çıkış yapar. Ajanlar çalışmaya devam eder.
password-title = Ekran parolası
password-intro = Başka bir makineden yapılan girişin kodla birlikte vermesi gereken bir parola belirleyin. Bu makinenin kendi tarayıcısına hiç sorulmaz.
password-intro-set = Ekran parolası ayarlı: başka bir makineden giriş bu parolayı ister. Değiştirmek diğer tüm pencerelerden çıkış yapar.
password-current = Mevcut parola
password-new = Yeni parola
password-again = Bir kez daha yazın
password-set = Parolayı ayarla
password-change = Parolayı değiştir
password-saved = Kaydedildi. Diğer tüm pencerelerden çıkış yapıldı.
password-refused = Bu parola doğru değil.
password-too-many = Çok fazla yanlış deneme. Bir süre bekleyip yeniden deneyin.
password-too-short = En az { $min } karakter kullanın.
password-mismatch = İki parola aynı değil.
password-failed = Hub'ınıza ulaşılamadı. Yeniden deneyin.
idle-lock = Kullanılmadığında bu pencereyi kilitle
idle-lock-off = Hiçbir zaman
idle-lock-minutes = { $minutes } dakika sonra
idle-lock-hint = Yalnızca bu tarayıcıda saklanır. Kullanılmayan pencere Kilitle gibi çıkış yapar; ajanlar çalışmaya devam eder. Yeniden giriş için kod (başka bir makineden ekran parolası da) ya da geçiş anahtarı gerekir.
idle-lock-hint-no-password = Yalnızca bu tarayıcıda saklanır. Kullanılmayan pencere Kilitle gibi çıkış yapar; ajanlar çalışmaya devam eder.
audit-action-sign-out = Çıkış yaptı
audit-action-password-set = Ekran parolasını ayarladı
audit-action-password-clear = Ekran parolasını kaldırdı
machines-title = Makineleriniz
machines-empty = Henüz bağlı makine yok.
machine-online = Çevrimiçi
machine-offline = Çevrimdışı
machine-last-seen = son görülme: { $when }
machine-agents =
    { $count ->
        [one] { $count } ajan çalışıyor
       *[other] { $count } ajan çalışıyor
    }
palette-go-machines = Makinelerimi göster
palette-language = Dil: { $name }
agent-tokens = { $count } token kullanıldı

start-title = Ajan başlat
start-where = Nerede
start-which = Hangi ajan
start-loading = Makineye soruluyor…
start-kinds-failed = Makine yanıt vermedi. Yeniden deneyin.
start-no-kinds = Bu makinede henüz ajan tanımlı değil.
start-no-projects = Bağlı hiçbir makinede proje yok. Birinde "agora node run --project YOL" çalıştırın.
start-not-installed = kurulu değil
start-not-installed-why = Bu makine "kurulu değil" olarak işaretlenen ajanların programlarını PATH içinde bulamıyor. Onları oraya kurun ya da düğümü bu programların çalıştığı bir terminalden başlatın.
start-not-installed-program = Bu makine { $program } programını PATH içinde bulamıyor. Onu oraya kurun ya da Agora'yı o makinede { $program } programının çalıştığı bir terminalden başlatın.
start-go = Başlat
start-cancel = Vazgeç
start-kind-unknown = Bu makine o ajanı tanımıyor.
start-limit = Makine kapasitesine ulaştı. Bir ajanı durdurup yeniden deneyin.
start-failed = Ajan başlamadı. Yeniden deneyin.
signin-passkey = Geçiş anahtarıyla giriş yap
signin-passkey-failed = Geçiş anahtarıyla giriş yapılamadı. Bunun yerine kod kullanın.
device-passkey-intro = Bu cihazda kodsuz giriş yapmak için bir geçiş anahtarı ekleyin.
device-passkey-add = Geçiş anahtarı ekle
device-passkey-added = Geçiş anahtarı eklendi. Bir dahaki sefere "Geçiş anahtarıyla giriş yap" seçeneğini kullanın.
device-passkey-failed = Geçiş anahtarı eklenemedi.
board-title = Ortak görevler ve dosyalar
board-open = açık
board-done = bitti
board-project-identity = Proje kimliği
board-project-folders = Makineler ve klasörler ({ $count })
messages-title = Son mesajlar
message-cross = makineler arası
console-agents = Ajanlar
console-activity = Etkinlik
console-wall-empty = Terminalini burada izlemek için soldan bir ajan seçin.
console-maximize = Tam boyut
console-hide = İzlemeyi bırak
console-no-messages = Ajanlar henüz birbirine yazmadı.
changes-title = Git'e kaydedilmemiş değişiklikler
changes-none = Git'e kaydedilmemiş değişiklik yok. Tüm çalışma alanları temiz.
changes-files = { $count ->
    [one] 1 dosya değişti
   *[other] { $count } dosya değişti
}
diff-title = { $name } neleri değiştirdi
diff-close = Kapat
diff-failed = Değişiklikler o makineden okunamadı.
changes-summary = { $count ->
    [one] 1 dosya
   *[other] { $count } dosya
}
changes-filter = Göster
changes-filter-all = Tümü
changes-filter-modified = Değişen
changes-filter-added = Eklenen
changes-filter-deleted = Silinen
changes-filter-renamed = Adı değişen
changes-status-modified = Değişti
changes-status-added = Eklendi
changes-status-deleted = Silindi
changes-status-renamed = Adı değişti
changes-badge-modified = D
changes-badge-added = E
changes-badge-deleted = S
changes-badge-renamed = A
changes-search = Ada göre süz
changes-layout = Görünüm
changes-layout-unified = Tek sütun
changes-layout-split = Yan yana
changes-wrap = Satırları kaydır
changes-wrap-long = Tamamı gösterilen uzun bir değişiklik her satırı tek sıra olarak çizilir, bu yüzden satır kaydırılmaz; yana kaydırın.
changes-refresh = Değişiklikleri yeniden oku
changes-list = Değişen dosyalar
changes-top-folder = Üst klasör
changes-no-match = Bu süzgece uyan değişen dosya yok.
changes-pick = Neyin değiştiğini görmek için soldan bir dosya seçin.
changes-renamed-from = önceki adı { $name }
changes-whole-file = Tüm dosya
changes-prev = Önceki dosya
changes-next = Sonraki dosya
changes-keys = sonraki ve önceki dosya
changes-binary = Bu dosya metin olmadığı için değişiklikleri gösterilmiyor.
changes-no-lines = Bu dosyanın yalnızca adı ya da izinleri değişti.
changes-secret = Değişti, içerik gizli
changes-no-eol = Dosyanın sonunda satır sonu yok
changes-gap = { $count ->
    [one] Değişmeyen 1 satırı göster
   *[other] Değişmeyen { $count } satırı göster
}
changes-gap-rest = Dosyanın kalanını göster
changes-gap-failed = Değişmeyen satırları göstermek için bu dosya okunamadı.
changes-retry = Yeniden dene
changes-stale = Değişiklikler okunduktan sonra dosya değişti, bu yüzden değişmeyen satırlar gösterilmiyor. Değişiklikleri yeniden okuyun.
changes-truncated = Değişiklikler 512 KiB'tan büyük olduğu için yalnızca ilk kısmı okundu. Listedeki son dosya yarıda kesik.
changes-truncated-count = Bu değişiklikler 512 KiB'tan büyük, bu yüzden yalnızca ilk { $count } dosya okundu. Makine { $total } değişen dosya sayıyor: en az { $min } tanesi burada listelenmiyor. Listedeki son dosya kesik.
files-truncated = Değişiklikler 512 KiB'tan büyük, bu yüzden kesimden sonraki dosyalarda değişiklik işareti olmayabilir.
files-truncated-count = Değişiklikler 512 KiB'tan büyük, bu yüzden kesimden sonraki dosyalarda değişiklik işareti yok. Makine { $total } değişen dosya sayıyor; { $count } tanesi işaretli.
changes-partial = Bu dosya yarıda kesik: değişiklikler tek seferde okunamayacak kadar büyüktü.
changes-eol = { $count } değişen satır yalnızca satır sonunda (LF veya CRLF) farklı; metin aynı.
changes-load-file = Yalnızca bu dosyayı oku
changes-load-file-failed = Bu dosya tek başına da okunamayacak kadar büyük.
changes-show-all = { $count ->
    [one] 1 satırın tümünü göster
   *[other] { $count } satırın tümünü göster
}
console-files = Dosyalar
files-title = { $name } dosyaları
files-tree = Klasörler ve dosyalar
files-pick = Okumak için soldan bir dosya seçin.
files-binary = Bu dosya metin olmadığı için gösterilmiyor.
files-too-large = Bu dosya okunamadı (çok büyük olabilir).
files-secret = Gizli tutuluyor: bir sır içeriyor olabilir
files-secret-row = { $name }, gizli tutuluyor
files-failed = Bu klasör listelenemedi.
files-filter = Dosyaları ada göre süz
files-only-changed = Yalnızca değişen dosyaları göster
files-changed-count = { $count ->
    [one] 1 değişen
   *[other] { $count } değişen
}
files-refresh = Klasörleri ve değişiklikleri yeniden oku
files-no-match = Uyan dosya yok. Henüz açılmamış klasörlerde aranmaz.
files-deleted = Bu dosya silindi; değişiklikleri inceleme ekranında.
files-empty = Bu klasör boş.
files-folder-changed = İçinde değişen dosyalar var
files-key-filter = dosyaları süz
files-key-find = dosyada ara
files-open-files = Açık dosyalar
files-close-tab = { $name } dosyasını kapat
files-path = Yol
files-lines = { $count ->
    [one] 1 satır
   *[other] { $count } satır
}
files-changed-lines = { $count ->
    [one] 1 satır değişti
   *[other] { $count } satır değişti
}
files-find = Dosyada ara
files-find-none = Eşleşme yok
files-find-at = { $used } / { $total }
files-find-prev = Önceki eşleşme
files-find-next = Sonraki eşleşme
files-find-close = Aramayı kapat
files-copy = Dosyanın tamamını kopyala
files-copied = Kopyalandı
files-reload = Bu dosyayı yeniden oku
files-mark-add = Son commit'ten bu yana eklendi
files-mark-mod = Son commit'ten bu yana değişti
files-mark-del = Son commit'ten bu yana burada satırlar silindi
filter-placeholder = Ajana, makineye veya projeye göre süz
filter-none = Eşleşen yok.
console-drag = Yerlerini değiştirmek için bu çubuğu başka bir terminalin üzerine sürükleyin
prompt-intro = Bu ajan terminalle değil, isteklerle çalışır. Ona ne yapacağını söyleyin.
prompt-label = Ne yapmalı?
prompt-send = Gönder
prompt-sent = Gönderildi.
prompt-failed = Gönderilemedi. Yeniden deneyin.
console-history = Geçmiş
history-title = { $name }: commit'ler
history-none = Burada henüz commit yapılmadı.
history-failed = Geçmiş o makineden okunamadı.
history-no-subject = (mesaj yok)
chat-empty = Henüz bir şey söylenmedi. Aşağıdan ona ne yapacağını söyleyin.
chat-log = Konuşma
chat-plan = Plan
chat-plan-steps =
    { $count ->
        [one] Plan · { $count } adım
       *[other] Plan · { $count } adım
    }
chat-plan-earlier =
    { $count ->
        [one] Önceki plan · { $count } adım
       *[other] Önceki plan · { $count } adım
    }
chat-steps-earlier =
    { $count ->
        [one] { $count } önceki adım
       *[other] { $count } önceki adım
    }
chat-working = Çalışıyor…
chat-copy = Kopyala
chat-copied = Kopyalandı
# Ajanın yapmak istediği iş, kendi başlığını vermediğinde. Nötr: iş sürüyor ya da reddedilmiş olabilir.
tool-read = Okuma
tool-edit = Düzenleme
tool-run = Komut
tool-search = Arama
tool-fetch = Web sayfası
tool-delete = Silme
tool-move = Taşıma
tool-think = Düşünme
tool-other = Adım
approval-full-request = İsteğin tamamı
notice-adapter-turn_failed = Ajanın sırası başarısız oldu (hata { $rpc_code }). Sağlayıcısı isteği reddetmiş olabilir; yeniden deneyin ya da başka bir model seçin. { $stderr_tail }
notice-adapter-turn_stopped = Ajan sırasını kendisi bitirdi ({ $stop_reason }).
notice-adapter-turn_empty = Ajan hiçbir şey söylemeden sırasını bitirdi. Sağlayıcısı isteği reddetmiş olabilir. { $stderr_tail }
notice-adapter-agent_exited = Ajan durdu (adım { $stage }, çıkış kodu { $exit_code }). Son çıktısı: { $stderr_tail }
notice-adapter-agent_refused = Ajan başlamayı reddetti (adım { $stage }, hata { $rpc_code }): { $detail }
notice-other = Ajan bu pencerenin gösteremediği bir şey bildirdi ({ $code }).
session-header = Ajan ayarları
session-elapsed-hint = Ne kadar süredir çalışıyor
session-mode = Mod
session-model = Model
session-not-reported = Bildirilmedi
session-mode-unasked = { $name } (sormadan)
session-context = Bağlam %{ $percent }
session-context-under-one = Bağlam %1'in altında
session-context-detail = Ajanın bağlamında { $size } jetondan { $used } jeton
session-change-failed = Ajan geçiş yapmadı.
session-confirm-title = Ajan sormadan çalışsın mı?
session-confirm-body = { $name } modunda ajan komut çalıştırır ve dosyaları size sormadan değiştirir.
session-confirm-yes = Yine de geç
session-confirm-no = Sormaya devam etsin
grant-title-typing = Bu ajana 1 saat boyunca yazmaya izin verilsin mi?
grant-title-start = Bu projede 1 saat boyunca ajan başlatmaya izin verilsin mi?
grant-title-other = Bu pencerenin 1 saat boyunca { $machine } üzerinde işlem yapmasına izin verilsin mi?
grant-explain = { $machine }, bu pencerenin bu işlemi yapmasına izin vermeden önce onayınızı ister. Verdiğiniz izin bu cihazdaki anahtarınızla imzalanır ve süresi dolunca kendiliğinden sona erer.
grant-allow = 1 saatliğine izin ver
grant-later = Şimdi değil
grant-failed = İzin iletilemedi. Yeniden deneyin.
audit-title = Hareketler
audit-intro = Bu hub'da kim ne yaptı? En yeni işlem en üstte gösterilir. Yalnızca hub sahibi görebilir.
audit-person = Kişi
audit-session = oturum { $id }
audit-session-hint = Her giriş ayrı bir oturumdur: aynı kişinin iki penceresi farklı oturumlar gösterir.
audit-empty = Henüz bir şey olmadı.
audit-owner-only = Hareketleri yalnızca hub sahibi görebilir.
audit-signed-out = Oturumunuz kapandı. Hareketleri görmek için yeniden giriş yapın.
audit-failed = Hareketler yüklenemedi.
audit-link = Hareketler
audit-outcome-done = Tamam
audit-outcome-sent = Gönderildi, henüz yanıt yok
audit-action-sign-in = Giriş yaptı
audit-action-user-add = Ekibe biri eklendi
audit-action-user-role = Rol değiştirildi
audit-action-user-remove = Ekipten biri çıkarıldı
audit-action-user-grant = Bir projeye erişim verildi
audit-action-user-revoke = Bir projeye erişim geri alındı
audit-action-agentstart = Ajan başlattı
audit-action-agentstop = Ajan durdurdu
audit-action-agentprompt = İstek gönderdi
audit-action-agentsetmode = Ajanın modunu değiştirdi
audit-action-terminalopen = Terminal açtı
audit-action-terminalinput = Terminale yazdı
audit-action-terminalresize = Terminali yeniden boyutlandırdı
audit-action-terminalsubscribe = Terminali izledi
audit-action-chatsubscribe = Bir konuşmayı açtı
audit-action-fileswrite = Dosya değiştirdi
audit-action-filesread = Dosya okudu
audit-action-fileslist = Dosyaları listeledi
audit-action-fileswatch = Dosyaları izledi
audit-action-gitdiff = Değişikliklere baktı
audit-action-gitstatus = Değişikliklere baktı
audit-action-gitlog = Geçmişe baktı
audit-action-worktreecreate = Çalışma alanı oluşturdu
audit-action-worktreeremove = Çalışma alanını kaldırdı
audit-action-approvalresolve = Bir onaya yanıt verdi
audit-action-taskupdate = Bir görevi değiştirdi
audit-action-nodepolicyget = Makinenin kurallarını okudu
audit-action-grantcreate = Makineye izin verdi
audit-action-grantlist = İzinlere baktı
audit-action-grantrevoke = Bir izni geri aldı
audit-action-projectlist = Projeleri listeledi
audit-action-agentlist = Ajanları listeledi
audit-action-agentkinds = Ajan türlerini listeledi
audit-action-usagereport = Harcamalara baktı

start-no-machines = Henüz çevrimiçi makine yok.
start-machine = Makine
start-project = Proje
start-other-folder = Başka bir klasör…
start-directory = Çalışma klasörü
start-directory-placeholder = Seçilen makinedeki tam klasör yolu
start-portable = Ortak proje kimliği oluştur
start-portable-explain = Proje klasörüne .agent-project.json dosyası ekler. Projeyi diğer makinelerle aynı grupta görmek için bu dosyayı Git ile veya kopyalayarak taşıyın. Mevcut kimlik dosyası korunur.
start-select-directory = Bu klasörü kullan
start-in-directory = Ajan bu klasörde çalışacak. Mevcut dosyalar ve dal korunur.
start-project-id = Proje kimliği: { $id }
start-separate-worktree = Ayrı bir çalışma kopyası oluştur
start-identity-change = Bu klasörü değişen proje kimliğiyle eşleştir
start-directory-unsupported = Seçilen klasörde ajan başlatmak için bu makineyi güncelleyin.
start-directory-failed = Klasör seçilemedi. Yolu ve erişim izinlerini kontrol edin.
start-directory-forbidden = Bu cihaz bu makinede başka projelerle sınırlı. Üzerinde çalıştığınız bir projenin klasörünü seçin ya da makinenin sahibine sorun.
workspace-shared = Ortak klasör · değişiklikler çalışma alanına ait
filter-project = Proje
filter-all-projects = Tüm projeler
filter-project-machines = { $name } · { $count } makine

start-directory-agent-unsupported = Bu ajan, ayarlarını yüklemek için ayrı bir çalışma kopyası gerektiriyor.

audit-action-directoryinspect = Bir klasör seçti
audit-action-agentlaunch = Bir klasörde ajan başlattı
audit-action-workspacequery = Çalışma alanının dosyalarına veya değişikliklerine baktı
audit-action-node-resynced = Bir makineyi yeniden bağladı; geçmişinin bir kısmı eksik

agent-address = Ajan adresi
agent-address-copy = Ajan adresini kopyala
address-copy = Kopyala
address-copied = Kopyalandı
address-copy-failed = Kopyalanamadı. Adresi seçip elle kopyalayın.
machine-identity = Makine kimliği: { $id }
machine-name-id = { $name } · { $id }

language-label = Dil
language-system = Tarayıcı dili
language-en = English
language-tr = Türkçe
language-not-saved = Dil bu ziyaret için değiştirildi. Tarayıcınız seçimi kaydedemedi.

# IDE tarzı pencere: başlık çubuğu, ajan ağacı, sekmeler, ayrıntı paneli, durum çubuğu.
ide-command = Ara veya bir komut çalıştır
ide-new-tab = Ajan başlat
ide-close-tab = { $name } sekmesini kapat
ide-hide-panel = Bu paneli gizle
ide-show-agents = Ajan listesini göster
ide-show-details = Ayrıntıları göster
ide-layout-single = Tek seferde bir terminal
ide-layout-grid = Terminaller yan yana
ide-details = Ayrıntılar
ide-welcome-title = Kaldığınız yerden devam edin
ide-welcome-running = Şu an çalışanlar
ide-welcome-none = Henüz çalışan bir ajan yok.
ide-welcome-search = Arama ve komutlar
ide-welcome-next = Sizi bekleyen sıradaki ajana geç
ide-add-here = { $machine } üzerinde ajan başlat
ide-toggle-machine = { $name } üzerindeki ajanları göster veya gizle
ide-status-agents =
    { $count ->
        [one] { $count } ajan
       *[other] { $count } ajan
    }
ide-status-needs =
    { $count ->
        [one] { $count } ajan sizi bekliyor
       *[other] { $count } ajan sizi bekliyor
    }
ide-folder-known = Bilinen klasörler
ide-folder-change = Klasörü değiştir
ide-folder-chosen = Çalışma klasörü
ide-agent-pick = Ajan
ide-options = Seçenekler
ide-copy-address = Adresi kopyala
ide-tab-changes = Değişiklikler
ide-tab-messages = Mesajlar
ide-more-tabs = Diğer sekmeler
ide-more-tabs-needs = gizli bir sekme sizi bekliyor
ide-more-tabs-failed = gizli bir sekme hatayla bitti
ide-more-tabs-working = gizli bir sekme çalışıyor
theme-label = Renk düzeni
theme-system = Sistemi izle
ide-shared = Paylaşılan
ide-memory = Bu makinedeki ajanların kullandığı bellek
network-hub = Hub'ınız
network-build = Bu makinede çalışan sürüm
network-never = hiç
network-heard = { $when } duyuldu
network-last-seen = son görülme: { $when }
network-online =
    { $total ->
        [one] { $total } makineden { $online } tanesi çevrimiçi
       *[other] { $total } makineden { $online } tanesi çevrimiçi
    }
network-terminals =
    { $count ->
        [one] { $count } terminal açık
       *[other] { $count } terminal açık
    }
terminal-size = Terminal yazı boyutu
terminal-smaller = Daha küçük yazı
terminal-larger = Daha büyük yazı
terminal-settings = Terminal ayarları
terminal-line-height = Satır yüksekliği
terminal-line-height-note = 1,0 gerçek bir terminalin satır yüksekliğidir
terminal-history = Tutulan geçmiş satırı
terminal-readable = Okunması zor renkleri okunur yap
terminal-copy-on-select = Seçtiğimi kopyala
terminal-gpu = Daha keskin blok ve çizgi çizimi; deneyin, yazı bozuk görünürse kapatın
terminal-jump-bottom = En yeniye git
terminal-find = Terminalde bul
terminal-find-next = Sonraki eşleşme
terminal-find-previous = Önceki eşleşme
terminal-find-close = Aramayı kapat
terminal-find-none = Eşleşme yok
terminal-copy-offer = Bu terminaldeki bir program şunu panonuza kopyalamak istiyor:
terminal-copy-offer-length = ({ $count } karakter)
terminal-copy-offer-copy = Kopyala
terminal-copy-offer-ignore = Yoksay
closed-section = Kapananlar
closed-clear = Kapananları temizle
closed-dismiss = Bu listeden kaldır
closed-forget = Herkes için kaldır
closed-ok = Tamamlandı
closed-failed = Hatayla bitti (çıkış kodu { $code })
closed-failed-signal = Hatayla bitti
closed-note = Terminali artık yok. Geride bıraktıkları aşağıda.
closed-machine = Makine
closed-folder = Klasör
closed-branch = Dal
closed-ended-label = Bitti
closed-earlier = bu pencere açılmadan önce
closed-ran-label = Çalışma süresi
closed-changes-label = Geride kalan
closed-changes-none = Kaydedilmemiş değişiklik yok
closed-tokens-label = Kullanılan
closed-review = Değişiklikleri incele
ide-resize = Paneli yeniden boyutlandır
start-browse = Klasörlere göz at
start-browse-up = Bir üst klasör
start-browse-close = Klasör listesini kapat
start-browse-empty = Burada klasör yok.
start-browse-more = Gösterilenden fazla klasör var. Daha ileri gitmek için yolu yazın.
start-browse-failed = Klasör listelenemedi.
start-browse-git = Bir Git projesi
closed-clear-everyone = Kapanan her ajanı herkes için kaldır
closed-forget-failed = Hub kaldırmadı: { $reason }
closed-reason-forbidden = bunu kaldırma yetkiniz yok
closed-reason-running = yeniden çalışıyor
closed-reason-gone = hub artık bunu tanımıyor
closed-reason-other = hub'a ulaşılamadı ya da reddetti

trust-hint = { $machine }, ajan başlatmadan önce bu tarayıcıya bir kez güvenmeli; böylece hub tek başına orada hiçbir program çalıştıramaz. { $machine } üzerinde bir terminalde bu komutu çalıştırın, gösterdiği parmak izinin "Bu cihaz" ile aynı olduğunu kontrol edin ve y yazın. Sonra yeniden Başlat'a basın:
trust-hint-binary = { $machine }, ajan başlatmadan önce bu tarayıcıya bir kez güvenmeli; böylece hub tek başına orada hiçbir program çalıştıramaz. Agora orada PATH üzerinde olmayan { $where } konumundan çalışıyor: o klasörde bir terminalde (Windows'ta PowerShell) bu komutu çalıştırın, parmak izinin "Bu cihaz" ile aynı olduğunu kontrol edin ve y yazın. Sonra yeniden Başlat'a basın:
trust-hint-file = { $machine }, ajan başlatmadan önce bu tarayıcıya bir kez güvenmeli; böylece hub tek başına orada hiçbir program çalıştıramaz. Oradaki Agora programı ({ $file }) PATH üzerinde değil: o dosyanın bulunduğu klasörde bir terminalde (Windows'ta PowerShell) bu komutu çalıştırın, parmak izinin "Bu cihaz" ile aynı olduğunu kontrol edin ve y yazın. Sonra yeniden Başlat'a basın:
trust-show-path = Yolun tamamını göster
trust-copy = Kopyala
trust-copied = Kopyalandı
interrupted-section = Makine kapanınca durdu
interrupted-hint = Bunlar makine durduğunda çalışıyordu.
interrupted-note = Makinesi durduğunda çalışıyordu. Devam Et, konuşmasını aynı klasörde sürdürür.
interrupted-note-new = Makinesi durduğunda çalışıyordu. Konuşması sürdürülemiyor; Yeniden Başlat aynı klasörde yeni bir konuşma açar.
interrupted-note-running = Makinenin yeniden başlamasından önceki işlemi hâlâ çalışıyor. Önce durdurun; sonra devam ettirilebilir. Ya da çalışmaya bırakın.
action-stop-old = Durdur
action-resume = Devam Et
action-start-again = Yeniden Başlat
resume-failed = Başlamadı. Yeniden deneyin.
resume-failed-folder = Klasörü yok ya da artık başka bir projeye ait, bu yüzden başlatılmadı.
resume-failed-parent = Onu başlatan ajan çalışmıyor. Önce onu devam ettirin.
resume-failed-session = Konuşması sürdürülemiyor. Yeniden Başlat'ı kullanın.
resume-failed-gone = O makine artık bunu sunmuyor.
resume-failed-unsupported = O makine henüz ajanları devam ettiremiyor. Güncelleyin.
resume-failed-still-running = Yeniden başlamadan önceki işlemi hâlâ çalışıyor. Önce durdurun.
resume-failed-offline = O makine çevrimdışı.
resume-failed-forbidden = Bu cihaz bu makinede başka projelerle sınırlı, bu yüzden bu ajanı başlatamaz.

# ADR-0059: launch options (manifest `[[launch.option]]` labels)
option-skip-permissions = Sormadan
setting-model = Model
setting-effort = Efor
option-ask = Önce sorsun
start-how-it-acts = Nasıl çalışsın
start-confirm-skip-body = "{ $name }" seçiliyken ajan, çalıştığı sürece komut çalıştırır ve dosyaları size sormadan değiştirir.
start-confirm-skip-yes = Sormadan başlat
start-does-not-ask-note = Bu ajan size sormadan çalışacak.
start-option-forbidden = Bu makine ajanların sormadan çalışmasına izin vermiyor. "Önce sorsun"u seçin.
start-option-confined-only = Bu makine bir ajanın sormadan çalışmasına yalnızca klasörüyle sınırlıyken izin veriyor; burada sınırlı olmazdı.
start-options-unsupported = Bu makine başlatma seçenekleri için fazla eski. Güncelleyin ya da "Önce sorsun"u seçin.
agent-does-not-ask = Onay istemeden çalışır
agent-does-not-ask-short = Serbest
agent-does-not-ask-explain = Size önce sormadan çalışacak şekilde başlatıldı
start-option-unconfirmed = Bu makineye göre o seçenek ajanın sormadan çalışmasını sağlıyor ve bu pencere önce size sormadı. Hiçbir şey başlatılmadı; başlatma penceresini yeniden açın.
start-option-invalid = Bu makine o başlatma seçeneğini sunmuyor. Hiçbir şey başlatılmadı; başlatma penceresini yeniden açın.
# Yerleşimler (ADR-0068): yan yana bölmeler, sekmeler, kayıtlı yerleşimler.
layout-menu = Yerleşim
layout-menu-back = Geri
layout-preset-1x1 = Sekmeli tek bölme
layout-preset-2x1 = Yan yana iki
layout-preset-1x2 = Alt alta iki
layout-preset-2x2 = Dört (2 × 2)
layout-preset-3x2 = Altı (3 × 2)
layout-preset-focus = Bir büyük, üç küçük
layout-maximize = Büyüt
layout-restore = Geri al
layout-save-as = Yerleşimi farklı kaydet…
layout-saved = Kayıtlı yerleşimler…
layout-more = Diğer
layout-export = Dışa aktar
layout-import = İçe aktar…
layout-reset = Yerleşimi sıfırla
layout-split-right = Sağa böl
layout-split-down = Aşağı böl
layout-close-pane = Bölmeyi kapat
layout-pane-menu = Bölme seçenekleri: { $name }
layout-move-to = Taşı…
layout-swap-with = Yer değiştir…
layout-move-out = Sekmelerden çıkar
layout-place-before = Yanına, önce
layout-place-after = Yanına, sonra
layout-place-above = Üstüne
layout-place-below = Altına
layout-place-tab = Sekme olarak
layout-show = Göster…
layout-follow-attention = Sırada sizi bekleyen
layout-follow-selection = Listede seçilen ajan
layout-follows-attention = Sizi bekleyeni izler
layout-follows-selection = Seçimi izler
layout-show-agent = { $name }
layout-show-agent-itself = Bu ajan
layout-show-agent-files = Bu ajanın dosyaları
layout-panel-agent = Ajan
layout-panel-empty = Boş bölme
layout-panel-unknown = Bilinmeyen panel “{ $id }”: bu pencere onu gösteremiyor.
layout-drag = Başka bir bölmenin üzerine sürükleyin: kenara bırakmak yanına koyar, ortaya bırakmak sekme yapar
layout-empty-title = Bu bölmede ne gösterilsin
layout-nothing-selected = Burada görmek için listeden bir ajan seçin.
layout-gone = Bu ajan artık yok.
layout-files-open = { $name } dosyalarına göz at
layout-files-none = Dosyalar, bu pencere makinelerinize bağlanınca gösterilir.
layout-no-terminal = Bu ajan bir sohbet; terminali yok.
layout-no-conversation = Bu ajanın sohbeti değil, terminali var.
layout-pane = Bölme: { $name }
layout-divider = Bölmeleri yeniden boyutlandır
layout-save-title = Yerleşimi kaydet
layout-save-name = Ad
layout-save-replaces = Bu adla kayıtlı yerleşimin yerine geçecek.
layout-cancel = Vazgeç
layout-save = Kaydet
layout-list-title = Kayıtlı yerleşimler
layout-list-empty = Henüz kayıtlı yerleşim yok. Yerleşim menüsünde “Yerleşimi farklı kaydet…” seçin.
layout-apply = Kullan
layout-rename = Yeniden adlandır
layout-delete = Sil
layout-delete-named = { $name } yerleşimini sil
layout-close = Kapat
layout-import-title = Yerleşim içe aktar
layout-import-paste = Bir yerleşim yapıştırın (JSON)
layout-import-file = Ya da bir dosya seçin
layout-import-do = İçe aktar
layout-error-json = Bu geçerli bir JSON değil.
layout-error-invalid = Bu, bu pencerenin okuyabileceği bir yerleşim değil.
layout-error-newer = Bu yerleşim daha yeni bir sürümle yapılmış. Kullanmak için güncelleyin.
layout-error-too-deep = Bu yerleşim çok derin iç içe (en çok { $max } düzey).
layout-error-too-many = Bu yerleşimde çok fazla bölme var (en çok { $max }).
layout-board-empty = Henüz ortak görev ya da dosya yok.

## Temalar (ADR-0068 2. basamak, docs/ux/themes.md)
theme-editor-title = Tema
theme-edit = Temayı düzenle
theme-follows-system = Sistemi izliyor: { $scheme } tema gösteriliyor. Her biri için bir tema seçin.
theme-forced = Her zaman { $scheme }. İki temanız arasında kendiliğinden geçmek için sistemi izleyin.
theme-for-light = Açık tema
theme-for-dark = Koyu tema
theme-name = Ad
theme-built-in = yerleşik
theme-built-in-hint = Yerleşik temalar değiştirilemez. Özelleştir, değiştirebileceğiniz bir kopya oluşturur.
theme-customize = Özelleştir
theme-copy-name = { $name } (benim)
theme-colours = Renkler
theme-shape = Yazı ve biçim
theme-readability = Okunabilirlik
theme-incomplete = Bazı değerler henüz geçerli değil; pencere son geçerli değerleri korur.
theme-contrast-ok = Tüm yazılar okunabilir (WCAG AA).
theme-contrast-not-applied = Uygulanmadı: bazı yazılar zor okunur.
theme-px = { $size } px
theme-import = İçe aktar…
theme-export = Dışa aktar
theme-delete = Temayı sil
theme-reset = Varsayılanlara dön
theme-import-hint = Temalar bu tarayıcıda saklanır. Başka bir tarayıcıda kullanmak için dışa aktarın; Varsayılanlara dön, Açık ve Koyu'yu geri getirir ve temalarınızı korur.
theme-name-dark = Koyu
theme-name-light = Açık
theme-name-high-contrast = Yüksek karşıtlık
theme-name-large-text = Büyük yazı
theme-name-slate = Arduvaz
theme-name-sand = Kum
theme-token-surface-0 = Arka plan
theme-token-surface-1 = Paneller
theme-token-surface-2 = Yükseltilmiş yüzeyler
theme-token-border = Çizgiler
theme-token-text = Yazı
theme-token-text-muted = İkincil yazı
theme-token-accent = Vurgu
theme-token-on-accent = Vurgu üzerindeki yazı
theme-token-state-working = Çalışıyor
theme-token-state-needs-you = Sizi bekliyor
theme-token-state-finished = Bitti
theme-token-state-idle = Boşta
theme-token-state-unknown = Bilinmiyor
theme-token-font-ui = Arayüz yazı tipi
theme-token-font-mono = Terminal ve kod yazı tipi
theme-token-text-16 = Yazı boyutu
theme-token-space-1 = Boşluk
theme-token-radius-xs = Köşeler: çok küçük
theme-token-radius-s = Köşeler: küçük
theme-token-radius-m = Köşeler: orta
theme-token-radius-l = Köşeler: büyük
theme-error-json = Bu dosya JSON değil.
theme-error-invalid = Bu bir Agora tema dosyası değil.
theme-error-newer = Bu tema Agora'nın daha yeni bir sürümüyle yapılmış.
theme-error-name = Temanın 1 ile { $max } karakter arasında bir adı olmalı.
theme-error-scheme = Tema "dark" (koyu) mu "light" (açık) mı olduğunu belirtmeli.
theme-error-unknown-field = "{ $field }" bir tema dosyasının parçası değil.
theme-error-unknown-token = "{ $token }" bir tema değeri değil.
theme-error-derived-token = "{ $token }", "{ $base }" değerinden türer: onun yerine "{ $base }" ayarlayın.
theme-error-bad-color = { $label } ("{ $token }") #1a2b3c gibi bir renk olmalı.
theme-error-bad-size = { $label } ("{ $token }") { $min } ile { $max } px arasında olmalı.
theme-error-bad-font = { $label } ("{ $token }") virgülle ayrılmış yazı tipi adlarından oluşmalı.
theme-error-contrast = { $bg } üzerinde { $fg } { $ratio }:1; okunabilir olması için en az { $min }:1 gerekir.
theme-error-full = { $max } temanız var; önce birini silin.
palette-theme-editor = Temayı düzenle
palette-theme = Tema: { $name }
palette-open-machine = Makine: { $name }
palette-open-project = Proje: { $name }
palette-settings = Ayarlar
device-appearance = Görünüm
device-appearance-intro = Bu tarayıcı { $name } temasını gösteriyor. Temalar yalnızca bu tarayıcıda saklanır.

## ADR-0067 2. basamak: telefon boyutunda pencere ve terminal tuş çubuğu.
phone-nav = Git
phone-search = Ara
phone-start = Başlat
phone-machines = Makineler
phone-back = Listeye dön
phone-offline = çevrim dışı
phone-no-agents = Bu makinede ajan yok.
phone-needs = Sizi bekleyenler · { $count }
phone-answer-below = Yanıtı ekranın altındaki düğmelerle verin.
phone-next = Sıradaki · { $count }
phone-untrusted = { $machine } bu cihaza henüz güvenmiyor; buradan verilen yanıtı reddeder.
key-bar = Terminal tuşları
key-esc = Esc
key-tab = Tab
key-ctrl = Ctrl
key-up = Yukarı
key-down = Aşağı
key-left = Sol
key-right = Sağ
key-paste = Yapıştır

# ADR-0073: eski makineler, makineleri unutmak
old-machines-title = { $count ->
    [one] Bir eski makine
   *[other] { $count } eski makine
}
old-machines-explain = Bir günden uzun süredir görülmedi. Yeni ajanlar için önerilmez. Yeniden bağlanan her zamanki gibi görünür.
old-machines-cleanup = Temizle…
forget-machine-action = Unut…
forget-machine-title = { $name } unutulsun mu?
forget-machines-title = { $count ->
    [one] Bir eski makine temizlensin mi?
   *[other] { $count } eski makine temizlensin mi?
}
forget-machines-explain = Unutulan makineler ve biten ajanları her pencereden kalkar. Yaptıkları geçmişte ve maliyetlerde kalır. Yeniden bağlanan makine geri gelir.
forget-machine-agents = { $count ->
    [one] o sırada bir ajan çalışıyordu; orada hâlâ çalışıyor olabilir
   *[other] o sırada { $count } ajan çalışıyordu; orada hâlâ çalışıyor olabilirler
}
forget-machines-refuse = Yeniden bağlanırlarsa reddet (her biri yeni bir katılım kodu ister)
forget-machines-cancel = Kalsın
forget-machines-confirm = { $count ->
    [one] Unut
   *[other] { $count } makineyi unut
}
forget-machine-reason-forbidden = Makineleri yalnızca hub'ın sahibi unutabilir.
forget-machine-reason-online = Yeniden bağlandı, bu yüzden unutulmadı.
forget-machine-reason-gone = Hub bu makineyi artık tanımıyor.
forget-machine-reason-other = Hub unutamadı. Yeniden deneyin.

# ADR-0066: bir türün aldığı model ve efor, başlatma penceresinde.
setting-default = Ajanın kendi varsayılanı
setting-default-is = Ajanın kendi varsayılanı ({ $value })
start-settings-summary = { $kind } şu ayarlarla başlayacak: { $settings }.
start-settings-unsupported = Bu makine, model veya eforla ajan başlatmak için fazla eski. Güncelleyin ya da ajanın kendi varsayılanını seçin.
start-setting-refused = Bu makine, bu ajan için o modeli veya eforu almıyor. Başka birini ya da ajanın kendi varsayılanını seçin.

# Ana sayfa bir filo konsolu olarak: kimin sizi beklediği, kimin çalıştığı, neyin bittiği, neyin hata verdiği, hangi makinelerin çevrimiçi olduğu.
fleet-panel = Genel durum
fleet-summary-label = Genel durum bir bakışta
fleet-filter-hint = Yalnızca şunları göster
fleet-failed = Hata verdi
fleet-machines =
    { $total ->
        [one] { $total } makineden { $online } tanesi çevrimiçi
       *[other] { $total } makineden { $online } tanesi çevrimiçi
    }
fleet-filter-active = Yalnızca “{ $name }” gösteriliyor.
fleet-filter-clear = Hepsini göster
fleet-filter-none = Burada gösterilecek bir şey yok.
fleet-attention-count = { $count ->
    [one] 1 şey sizi bekliyor
   *[other] { $count } şey sizi bekliyor
}
fleet-asked = { $when } soruldu
fleet-waiting-in-terminal = Terminalinde sizi bekliyor
fleet-asks-start = Başka bir ajan başlatmak istiyor
fleet-allow-for = İzin ver: { $name }
fleet-deny-for = Reddet: { $name }
fleet-review = İncele ve karar ver
fleet-review-for = İncele ve karar ver: { $name }
fleet-open-for = { $name } ajanını aç
fleet-group-by = Gruplama
fleet-group-machine = Makine
fleet-group-project = Proje
fleet-no-project = Projesiz
fleet-group-kind = Ajan
fleet-group-state = Durum
fleet-no-kind = Bilinmeyen ajan
fleet-same-project =
    { $count ->
        [one] Aynı proje · { $count } makine
       *[other] Aynı proje · { $count } makine
    }
fleet-same-project-on = Şurada açık: { $machines }
fleet-project-changing =
    { $count ->
        [one] Bu projede { $count } makinede değişiklik var.
       *[other] Bu projede { $count } makinedeki ajanların değişiklikleri var. Aynı dosyaları düzenlemediklerinden emin olun.
    }
fleet-offline-since = Çevrimdışı. Son görülme: { $when }.
fleet-offline-note = Ajanlar orada çalışmaya devam eder; yaptıkları makine yeniden bağlanınca burada görünür.
fleet-no-agents-here = Burada henüz ajan yok.
fleet-start-on = { $machine } üzerinde ajan başlat
fleet-tile-asks = İstediği: { $what }
fleet-tile-stopped = Makinesiyle birlikte durdu
fleet-tile-state-for = { $time } süredir: { $state }
fleet-tile-ended = { $when } bitti
fleet-empty-title = İlk makinenizi bağlayın
fleet-empty-text = Makine ajanlarınızı çalıştırır ve katılır katılmaz burada görünür. Agora'nın başladığı makinede bir katılma kodu alın, sonra eklemek istediğiniz makinede ikinci komutu çalıştırın.
fleet-empty-step-code = Agora'nın başladığı makinede
fleet-empty-step-join = Eklenecek makinede
fleet-empty-ca = ca.pem, o makinedeki ~/.agora/hub/ca.pem dosyasıdır.
fleet-empty-machines = Makinelerinizi görün
fleet-firstrun-title = Makineleriniz hazır. İlk ajanı başlatın.
fleet-firstrun-text = Bir makine ve proje klasörü seçin, ajan orada başlasın. Burada görünür ve sizi gerektiğinde çağırır.
fleet-firstrun-cli = Ya da o makinedeki bir terminalden
fleet-firstrun-offline = Şu an tüm makineler çevrimdışı. Birinde Agora'yı başlatın, burada görünsün.
fleet-offline-title = Agora'ya ulaşılamıyor
fleet-offline-text = Ajanlarınız makinelerinde çalışmaya ve yaptıklarını kaydetmeye devam ediyor. Bağlantı geri gelir gelmez yeniden burada görünürler.
fleet-offline-stale = Agora ile bağlantı kesildi. Gördükleriniz bu pencerenin bildiği son durum ve eski olabilir. Ajanlar makinelerinde çalışmaya devam ediyor.
fleet-loading = Genel durum yükleniyor…
fleet-exit-code = Çıkış kodu { $code }
fleet-attention-more = { $count } tane daha göster
fleet-attention-less = Daha az göster
fleet-stale-no-answer = Bağlantı kesikken yanıt verilemez.
fleet-same-ask = { $machine } üzerinde { $count } { $kind } ajanı aynı şeyi istiyor
fleet-review-each = Tek tek incele
fleet-collapse = Tek satır olarak göster
fleet-same-ask-any = { $machine } üzerinde { $count } ajan aynı şeyi istiyor
attention-who = { $machine } üzerinde { $name }
attention-who-only = { $name }
attention-ask-review = Yanıtlamadan önce bunun tamamı okunmalı. Karar vermek için incele.
attention-start = { $name } başka bir ajan başlatmak istiyor. Klasörü, istemi ve ayarları tam kartta okunur.
attention-pair = Onaylamadan önce, tam kartta numarasını ekranındaki numarayla karşılaştır.
attention-later = Sonra
attention-review = İncele
attention-more = { $count ->
    [one] Bundan sonra 1 şey daha sana ihtiyaç duyuyor
   *[other] Bundan sonra { $count } şey daha sana ihtiyaç duyuyor
}
attention-banner = { $count ->
    [one] 1 şey sana ihtiyaç duyuyor
   *[other] { $count } şey sana ihtiyaç duyuyor
}
attention-settings-title = Sana ihtiyaç var
attention-settings-intro = Bir ajan takıldığında bu pencerenin dikkatini nasıl çektiği. Sana ihtiyaç var listesi ve sayıları her zaman görünür kalır. Yalnızca bu tarayıcıda saklanır.
attention-mode-modal = Pencerenin üstünde bir kart
attention-mode-banner = Yalnızca küçük bir şerit
attention-mode-off = Ek bir şey yok
attention-mode-hint = Kart yalnızca bu pencere odaktayken ve bir terminalde ya da alanda yazmıyorken açılır; aksi halde bekler ve bir şerit onu gösterir. Senin yerine asla yanıt vermez.
attention-notify-title = Pencere arka plandayken bildirimler
attention-notify-enable = Bildirimleri aç
attention-notify-off = Bildirimleri kapat
attention-notify-on = Bildirimler açık. Yalnızca bir ajanın sana ihtiyaç duyduğunu söylerler.
attention-notify-names = Ajanın ve makinenin adını ekle
attention-notify-blocked = Bu tarayıcı bu sayfa için bildirimleri engelliyor. Açmak için tarayıcının site ayarlarından izin ver.
attention-notify-unsupported = Bu tarayıcı bildirim gösteremiyor.
notify-title = Sana ihtiyaç var
notify-body = { $count ->
    [one] Bir ajanın sana ihtiyacı var
   *[other] { $count } ajanın sana ihtiyacı var
}
notify-body-named = { $name } sana ihtiyaç duyuyor
notify-body-named-on = { $machine } üzerinde { $name } sana ihtiyaç duyuyor
attention-sent = Yanıt gönderildi. Bu kart kalırsa İncele ile aç.
attention-notify-names-warn = Bu adlar ajanlardan ve makinelerden gelir; bildirim kilitli ekranda görünebilir. Düz metin olmayan bir ad bildirime konmaz.

# Usage (no money: tokens, context and time)
ide-tab-cost = Kullanım
ide-tab-usage = Kullanım
usage-empty = Henüz gösterilecek bir şey yok. Bir ajan çalışınca kullanım burada görünür.
usage-total-tokens = { $count } token kullanıldı
usage-total-time = { $time } ajan süresi
usage-agents = { $count } ajan
usage-by-machine = Makineye göre
usage-by-project = Projeye göre
usage-context = Bağlam %{ $percent }
usage-limits-title = Plan limitleri
usage-limits-empty = Ajan bildirdiğinde plan limitleri burada görünür.
usage-reported-toggle = Ajanların maliyet hakkında bildirdikleri
usage-reported-note = Ajan tarafından bildirildi, planınızı yansıtmayabilir.
usage-reported-row = { $name }: { $amount }
