# Web istemcisi metinleri. Arayüz sade sözcükler kullanır, iç terimler kullanmaz.

app-name = Agora
home-title = Ana sayfa
gallery-title = Bileşen galerisi
attention-title = Sizi bekleyenler
attention-empty = Her şey sakin. Sizi bekleyen yok.
fleet-title = Ajanlarınız
fleet-empty = Henüz ajan yok. Bir ajan başlatın, burada görünsün.
fleet-watch-empty = Henüz çalışan ajan yok.

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
link-connecting = Agora’nıza bağlanılıyor…
link-offline = Agora’nıza ulaşılamıyor. Yeniden deneniyor…
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
# Yeniden tasarım PR 4: bir isteğin başlığı isteğin kendisidir, pencerenin sözleriyle, kapalı bir tür kümesinden (başka her şey
# approval-title). Komut, dosya veya adres altındaki satırda, bütün olarak kalır.
approval-ask-run = { $name } bir komut çalıştırmak istiyor
approval-ask-edit = { $name } bir dosyayı değiştirmek istiyor
approval-ask-read = { $name } bir dosyayı okumak istiyor
approval-ask-fetch = { $name } bir web adresini açmak istiyor
approval-ask-delete = { $name } bir şeyi silmek istiyor
approval-ask-start = { $name } başka bir ajan başlatmak istiyor
approval-open-agent = { $name } ajanını aç
approval-not-trusted = Bu cihaza o makine henüz güvenmiyor. "Bu cihaz" sayfasını açın ve gösterdiği komutu makinede çalıştırın.
approval-failed = Yanıt iletilemedi. Yeniden deneyin.
approval-no-answer = Ajan yanıtınızı zamanında almadı. İstek hâlâ açık: biraz bekleyip yeniden yanıtlayın.
approval-agent-exited = Ajan durdu, bu yüzden yanıtınız ona ulaşmadı. Ajanı yeniden başlatın.
approval-forbidden = Bu cihazdan bu proje için yanıt veremezsiniz: orada başka projelerle sınırlı. Bu projede çalışan birine sorun.
approval-role = Rolünüz bu proje için yanıt vermenize izin vermiyor. Sahibine ya da bu projede çalışan birine sorun.
approval-no-longer-valid = Bu istek artık geçerli değil. İlgili ajan sona erdi veya yeniden başladı.
approval-ran-out = Bu isteğin süresi doldu. Ajanın denetimlerinden yeniden isteyin.
# ADR-0101: "Hep izin ver", ajanın hatırladığı bir yanıt. Oturum sözleri ADR'nindir; hiçbir tür henüz böyle işaretli değil, pencere "-kept" sözlerini gösterir.
approval-allow-always = Bu oturum boyunca hep izin ver
approval-allow-always-note = { $name } aynı istek saydığı şeyler için, sona erene kadar bir daha sormaz. Neyin kapsandığına kendisi karar verir.
approval-always-only = Bu ajan bu istek için tek seferlik izin sunmuyor: izin verirsen oturumunun geri kalanı için izin vermiş olursun.
approval-allow-always-kept = Hep izin ver
approval-allow-always-note-kept = { $name } bu yanıtı hatırlar ve aynı istek saydığı şeyler için bir daha sormaz. Neyin kapsandığına ve yanıtı ne kadar tutacağına kendisi karar verir; bu, bu oturumdan uzun sürebilir.
approval-always-only-kept = Bu ajan bu istek için tek seferlik izin sunmuyor: izin verirsen ajan yanıtı hatırlar, belki bu oturumdan sonra da.
approval-allow-always-label = Kendi sözleriyle: “{ $label }”
approval-scope-forbidden = Bu makine ajanların sormadan iş yapmasına izin vermiyor; yalnızca bu istek için izin verilebilir.
approval-scope-unavailable = Ajan bu istek için bu yanıtı sunmuyor. Hiçbir şey gönderilmedi; yeniden yanıtla.
approval-adapter-outdated = Bu ajanı Agora’nın eski bir sürümü başlattı; tek seferlik bir izni hatırlayabilir. Bu makine yalnızca tek seferlik yanıtlara izin veriyor: ajanı yeniden başlat, sonra yanıtla.
skip-approval-title = { $name } sormadan çalışsın mı?
skip-approval-body = Siz onay vermeden komut çalıştırır ve dosyaları değiştirir; bu, sona erene veya siz geri alana kadar sürer.
skip-approval-machine = Makine
skip-approval-setting = Ayar
skip-approval-choice = Geçilecek değer
skip-approval-scope = Yalnızca bu ajanın oturumunu kapsar.
skip-approval-other-window = Bu pencere makinenin bunu sunduğunu henüz duymadı. Burada yanıtlamak için ajanın denetimlerinden yeniden seçin.
skip-approval-unknown-title = Bu pencerenin anlamadığı bir istek
skip-approval-unknown = Yanıtlamak için pencerenizi güncelleyin. Hiçbir şey imzalanmadı.
skip-approval-dismiss = Kapat
skip-approval-read-first = Önce okuyun, sonra izin verebilirsiniz.
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
start-approval-more-short = Bunun gibi daha fazla
start-approval-no-grant = Yalnızca bu başlatmaya izin verilebilir: bu tür kendisine verilen her programı çalıştırır, bu yüzden hiçbir zaman sormadan başlatılmaz.
start-approval-settings-once = Yalnızca bu başlatmaya izin verilebilir: kendi modeli veya eforuyla başlatma her zaman sorulur.
start-approval-options-once = Yalnızca bu başlatmaya izin verilebilir: yanıtınız bu başlatmayı yalnızca burada gösterilen başlatma seçenekleriyle kapsar ve böyle her başlatma yeniden sorulur.
start-approval-does-not-ask = İzin verirseniz bu yardımcı, çalıştığı sürece size sormadan komut çalıştırır ve dosyaları değiştirir.
start-approval-more = { $name } çalıştığı sürece bu klasörde, yeniden sormadan en fazla { $count } { $kind } ajanı
start-approval-nested = Yardımcıları da yardımcı başlatmak isteyebilir (her birini siz yanıtlarsınız)
start-approval-remote-title = { $from } üzerindeki { $agentKind }, burada ({ $machine }) bir { $kind } ajanı başlatmak istiyor
start-approval-remote-note = Başka bir makine soruyor: klasör bu makinede ve { $from } başlattığı ajanı izleyebilir.
start-approval-remote-more = { $from } makinesinden gelen benzer başlatmalar burada sormadan başlar: { $where }, aynı anda en fazla { $count } { $what }, { $time }. İstediğiniz zaman geri alabilirsiniz.
start-approval-remote-what-kind = { $kind } ajanı
start-approval-remote-what-any = bu makinenin çalıştırdığı herhangi türde ajan
start-approval-remote-where-folder = bu klasörde
start-approval-remote-where-below = bu klasörde veya altındaki herhangi bir klasörde
start-approval-remote-until-run = soran çalışma bitene kadar (en fazla 24 saat)
start-approval-remote-until-hours = { $hours } saat boyunca
start-approval-remote-any-kind = Yalnızca bu tür değil, bu makinenin çalıştırdığı her tür
start-approval-remote-below = Bu klasörün altındaki klasörler de
start-approval-remote-for = Süre
start-approval-remote-for-hours = { $hours } saat
start-approval-remote-for-run = Çalışması bitene kadar (en fazla 24 sa)
start-approval-remote-clock = O makine yanıtı reddetti. Saati bu cihazınkinden ileride olabilir: "Çalışması bitene kadar" veya daha uzun bir süre seçip yeniden yanıtlayın.
start-approval-remote-nested = Başlattığı ajanlar da, hangi makineden olursa olsun, sorabilir (her biri bu izin içinde)
approval-review = İncele
approval-review-for = { $name } isteğinin tamamını oku
approval-review-close = Kapat
start-summary-outside = kendi klasörünün dışında
start-summary-more = { $count ->
    [one] 1 satır daha
   *[other] { $count } satır daha
}
helper-started-by = { $name } tarafından başlatıldı
helper-grants = Yardımcı izinleri
helper-grant-line = Sormadan hâlâ { $count } { $kind } başlatabilir, klasör:
helper-grant-revoke = Geri al
helper-grant-remote-line = { $machine } üzerinde: sormadan aynı anda en fazla { $count } { $kind } ({ $n } boş), klasör:
helper-grant-any-kind = her türden ajan
helper-grant-held-line = { $from } burada sormadan aynı anda en fazla { $count } { $kind } başlatabilir ({ $n } boş), klasör:
helper-grant-from = { $machine } üzerindeki { $agent }
helper-grants-failed = Yardımcı izinleri okunamadı ya da değiştirilemedi.
device-title = Bu cihaz
device-name = Cihaz adı
device-key = Cihaz anahtarı
device-link = Ayarlar
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
device-removed = Kaldırıldı
device-waiting = Onayınızı bekliyor
device-trusted-on = Güvenilen makineler: { $machines }
device-trusted-nowhere = Hiçbir makinede güvenilmiyor
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
pair-approve = Numaralar aynı…
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

signin-title = Agora’nıza giriş yapın
signin-intro = Agora’nın başladığı makinede bu komutu çalıştırın, sonra gösterdiği kodu buraya yazın.
signin-code = Kod
signin-action = Giriş yap
signin-refused = Bu kod kabul edilmedi. Yanlış, süresi dolmuş veya daha önce kullanılmış olabilir.
signin-too-many = Buradan çok fazla yanlış deneme yapıldı. Bir süre bekleyip yeniden deneyin.
signin-failed = Agora’nıza ulaşılamadı. Yeniden deneyin.
signin-password = Ekran parolası
signin-password-hint = Başka bir makineden giriş yaptığınız için Agora’nız ekran parolasını da istiyor.
signin-refused-password = Giriş yapılamadı. Kod yanlış, süresi dolmuş veya kullanılmış olabilir; ya da parola yanlış olabilir.
lock-now = Kilitle
lock-now-hint = Bu pencereden çıkış yapar. Ajanlar çalışmaya devam eder.
password-title = Ekran parolası
password-current = Mevcut parola
password-new = Yeni parola
password-again = Bir kez daha yazın
password-set = Parolayı ayarla
password-change = Parolayı değiştir
password-saved = Kaydedildi. Diğer tüm pencerelerden çıkış yapıldı.
password-refused = Bu parola doğru değil.
password-too-many = Çok fazla yanlış deneme. Bir süre bekleyip yeniden deneyin.
password-busy = Agora'nız şu anda yoğun. Birazdan yeniden deneyin.
password-too-short = En az { $min } karakter kullanın.
password-mismatch = İki parola aynı değil.
password-failed = Agora'nıza ulaşılamadı. Yeniden deneyin.
idle-lock = Kullanılmadığında bu pencereyi kilitle
idle-lock-off = Hiçbir zaman
idle-lock-minutes = { $minutes } dakika sonra
idle-lock-hint = Yalnızca bu tarayıcıda saklanır. Kullanılmayan pencere Kilitle gibi çıkış yapar; ajanlar çalışmaya devam eder. Yeniden giriş için kod (başka bir makineden ekran parolası da) ya da geçiş anahtarı gerekir.
idle-lock-hint-no-password = Yalnızca bu tarayıcıda saklanır. Kullanılmayan pencere Kilitle gibi çıkış yapar; ajanlar çalışmaya devam eder.
audit-action-sign-out = Çıkış yaptı
audit-action-password-set = Ekran parolasını ayarladı
audit-action-password-clear = Ekran parolasını kaldırdı
machines-title = Makinelerim
machines-empty = Henüz bağlı makine yok.
machine-online = Çevrimiçi
machine-offline = Çevrimdışı
machine-last-seen = son görülme: { $when }
# The Agents list: an offline machine's row ends with how long it has been away ("offline · 2 hr").
explorer-offline-for = çevrimdışı · { $when }
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
start-not-installed-why = Bu makine bu ajanın programını PATH içinde bulamıyor. Onu oraya kurun ya da Agora'yı o makinede programın çalıştığı bir terminalden başlatın.
start-not-installed-program = Bu makine { $program } programını PATH içinde bulamıyor. Onu oraya kurun ya da Agora'yı o makinede { $program } programının çalıştığı bir terminalden başlatın.
start-group-installed = Kurulu
start-group-missing = Kurulu değil
start-group-untested = Denenmedi
start-runs-terminal = Terminal
start-runs-chat = Sohbet
# Yeni ajanın Ana sayfada ve sekmelerde alacağı ad: türü, sonra başlarken seçilen bir renk ($name "codex · …" biçimindedir).
start-preview = { $name } olarak görünecek; rengi başlarken seçilir.
start-go = Başlat
start-cancel = Vazgeç
start-kind-unknown = Bu makine o ajanı tanımıyor.
start-program-missing = O makine bu ajanın programını PATH içinde bulamıyor. Programı oraya kurun ya da Agora'yı o makinede programın çalıştığı bir terminalden başlatın.
start-limit = Makine kapasitesine ulaştı. Bir ajanı durdurup yeniden deneyin.
start-failed = Ajan başlamadı. Yeniden deneyin.
signin-passkey = Geçiş anahtarı kullan
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
board-recently-accepted = Son kabul edilenler
board-accepted-open = Panoyu aç
board-open-view = Panoyu aç ({ $count })
board-view-title = Pano: { $name }
board-view-empty-title = Henüz görev yok
board-view-empty-hint = Ajanların bu proje için eklediği görevler burada görünür.
board-col-backlog = Bekleyen
board-col-doing = Yapılıyor
board-col-done = Bitti
board-col-backlog-empty = Bekleyen yok.
board-col-doing-empty = Devam eden yok.
board-col-done-empty = Henüz biten yok.
board-task-untitled = Adsız görev
board-card-parent-task = Üst görev: { $title }
board-card-subtask-progress = { $total } alt görevin { $accepted } tanesi kabul edildi
board-card-owner = Ajan: { $name }
board-handoff-pending = { $name } için görev devri bekliyor
board-handoff-delivered = Görev devri { $name } alıcısına ulaştı
board-handoff-failed = { $name } için görev devri başarısız oldu
board-handoff-unresolved = { $name } için görev devri doğrulanamadı
board-handoff-acknowledged = { $name } için doğrulanamayan devir görüldü
board-handoff-acknowledge = Doğrulanamayan devri gördüm
board-handoff-acknowledge-failed = Devir görüldü olarak işaretlenemedi. Tekrar dene.
board-handoff-history = Devir geçmişi
board-card-holds = Üzerinde çalıştığı dosyalar ({ $count })
board-card-more-paths = ve { $count } tane daha
board-show-more = Daha fazla göster ({ $count } / { $total })
# ADR-0095 adım 2: Bekleyen bir görevi bir ajana vermek.
give-action = Bir ajana ver
give-action-label = “{ $title }” görevini bir ajana ver
give-title = Bir ajana ver
give-pick-label = Bu görevi alabilecek ajanlar
give-pick-hint = { $project } projesinde çalışan ajanlar. Yardımcılar, onları başlatan ajanın altında listelenir.
give-none = { $project } projesinde, görev alabilen bağlı bir makinede çalışan ajan yok. Önce bir ajan başlatın.
give-row-detail = { $machine } üzerinde { $kind }
give-row-helper = Yardımcı · { $machine } üzerinde { $kind }
give-confirm-title = Bu görev { $agent } ajanına verilsin mi?
give-field-task = Görev
give-field-agent = Ajan
give-field-kind = Tür
give-field-machine = Makine
give-field-project = Proje
give-confirm-no-ask = { $agent } onay istemeden çalışır: bu görevde dosyaları değiştirmeden veya komut çalıştırmadan önce size sormaz.
give-confirm-body = Ajan görevi bir mesaj olarak alır. O sırada bir turun ortasındaysa görev, tur bitene kadar bekler. Ajan kendi ayarlarını korur ve onayınızı gerektiren her şey yine size sorulur.
give-send = Görevi ver
give-sending = Gönderiliyor…
give-back = Geri
give-cancel = Vazgeç
give-sent = { $agent } ajanına gönderildi.
give-queued = { $agent } ajanına gönderildi. Ajanın sürmekte olan turu bitene kadar bekler.
give-done = Tamam
give-other-agent = Başka bir ajan seç
give-failed = Görev gönderilemedi. Yeniden deneyin.
give-refused-paste = Bu ajanın terminal programı yapıştırılan bir görevi alamıyor. Başka bir ajan seçin ya da görevi ona kendiniz yazın.
give-refused-waiting = Bu ajanın, turunun bitmesini bekleyen bir görevi zaten var. Bekleyin ya da başka bir ajan seçin.
give-refused-limited = Son bir dakikada ajanlara { $limit } görev verdiniz. Biraz bekleyip yeniden deneyin.
give-refused-grant = Bu makine önce izninizi istiyor. Açılan pencerede izin verin, görev gönderilir.
give-refused-offline = O makine şu anda bağlı değil.
give-refused-forbidden = Bu ajana görev verme izniniz yok.
give-target-gone = O ajan durdu, kapatıldı ya da bu projeden ayrıldı. Hiçbir şey gönderilmedi; başka bir ajan seçin.
give-grant-declined = Makinenin izni verilmedi, bu yüzden hiçbir şey gönderilmedi. Yeniden deneyebilirsiniz.
# ADR-0095 adım 3: Bir pano görevi için yeni bir ajan başlatmak, sonra görevi ona vermek.
give-start-new = Bu görev için yeni bir ajan başlat
give-none-start = { $project } projesinde, görev alabilen bağlı bir makinede çalışan ajan yok. Bu görev için bir ajan başlatın.
give-start-waiting = Ajan başladı. Burada görünür görünmez sonraki adım açılır.
give-start-unnamed = Ajan başladı, ancak makinesi hangisi olduğunu bildirmedi. Listede göründüğünde onu seçin.
give-queued-new = { $agent } ajanına gönderildi. Ajan hazır olana ve sürmekte olan turu bitene kadar bekler.
start-for-task = { $project } projesindeki “{ $title }” görevi için
start-for-task-other-project = Bu klasör { $project } projesine ait değil. Görev yalnızca kendi projesinde çalışan bir ajana verilebilir: projenin klasörlerinden birini seçin.
give-none-plain = { $project } projesinde, görev alabilen bağlı bir makinede çalışan ajan yok.
give-start-late = Ajan henüz görünmedi. Geri dönün ve göründüğünde onu listeden seçin.
give-machine-unsupported = O ajanın makinesi henüz görev alamıyor; güncellenmesi gerekebilir. Hiçbir şey gönderilmedi.
message-cross = makineler arası
message-via = bildiren: { $machine }
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
filter-short = Süz
filter-none = Eşleşen yok.
console-drag = Yerlerini değiştirmek için bu çubuğu başka bir terminalin üzerine sürükleyin
prompt-intro = Bu ajan terminalle değil, isteklerle çalışır. Ona ne yapacağını söyleyin.
prompt-intro-watching = Bu ajan terminalle değil, isteklerle çalışır.
prompt-label = Ne yapmalı?
prompt-send = Gönder
prompt-sent = Gönderildi.
prompt-failed = Gönderilemedi. Yeniden deneyin.
prompt-no-answer = Ajan bunu aldığını onaylamadı. Biraz bekleyin; bir şey olmazsa yeniden gönderin.
prompt-agent-exited = Ajan durdu, bu yüzden bunu almadı. Ajanı yeniden başlatın.
console-history = Geçmiş
history-title = { $name }: commit'ler
history-none = Burada henüz commit yapılmadı.
history-failed = Geçmiş o makineden okunamadı.
history-no-subject = (mesaj yok)
chat-empty = Henüz bir şey söylenmedi. Aşağıdan ona ne yapacağını söyleyin.
chat-empty-watching = Henüz bir şey söylenmedi.
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
notice-adapter-no_answer = Ajan bu isteği { $waited_ms } ms içinde aldığını onaylamadı. Hâlâ üzerinde çalışıyor olabilir.
notice-adapter-queue_dropped = Bekleyen { $count } istem gönderilmedi: sıra kuyruk atılarak iptal edildi, ajan sona erdi ya da bir istem gönderilemedi.
notice-adapter-output_dropped = Agora yeniden başlarken ajan çalışmaya devam etti. O sırada söylediklerinin { $bytes } baytı kaybedildi; bu yüzden konuşmanın o zamana ait bir kısmı eksik olabilir.
notice-adapter-restarting = Ajan beklenmedik şekilde durdu. Agora onu yeniden başlatıyor (deneme { $attempt }); konuşma, mümkün olduğunca kaldığı yerden sürer.
notice-adapter-gave_up = Ajan { $restarts } yeniden başlatmadan sonra yine beklenmedik şekilde durdu (son çıkış kodu { $exit_code }), bu yüzden Agora onu yeniden başlatmayı bıraktı. Devam etmek için yeni bir ajan başlatın.
notice-adapter-gave_up-no_code = Ajan { $restarts } yeniden başlatmadan sonra yine beklenmedik şekilde durdu, bu yüzden Agora onu yeniden başlatmayı bıraktı. Devam etmek için yeni bir ajan başlatın.
notice-other = Ajan bu pencerenin gösteremediği bir şey bildirdi ({ $code }).
notice-approval-always_only_forbidden = Ajan “{ $action }” için izin istedi ve yalnızca hatırlayacağı bir yanıt sundu. Bu makine yalnızca tek seferlik yanıtlara izin veriyor, bu yüzden reddedildi.
notice-adapter-permission_unanswerable = Ajan “{ $action }” için izin istedi ama izin vermenin bir yolunu sunmadı, bu yüzden reddedildi.
notice-chat-history_not_saved = Bu konuşma çalıştığı makinede kaydedilemedi (diski dolu olabilir). Ajan çalışmaya devam ediyor, ancak bundan sonraki adımlar Agora yeniden başladıktan sonra eksik olabilir.
notice-agent-skip_permissions_refused = { $setting ->
    [config] Bu ajanın kendi ayarlarından biri, size sormadan komut çalıştırmasına ve dosyaları değiştirmesine izin veriyor.
    [model] Bu ajanın kullandığı model, size sormadan komut çalıştırmasına ve dosyaları değiştirmesine izin veriyor.
   *[mode] Bu ajanın modu, size sormadan komut çalıştırmasına ve dosyaları değiştirmesine izin veriyor.
} { $policy ->
    [confined_only] Bu makine buna yalnızca klasörüyle sınırlı ajanlar için izin veriyor, bu yüzden Agora onu durdurdu. Ajandaki bu ayarı soran bir ayarla değiştirip yeniden başlatın.
   *[forbid] Bu makine buna izin vermiyor, bu yüzden Agora onu durdurdu. Ajandaki bu ayarı soran bir ayarla değiştirip yeniden başlatın.
}
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
session-waiting-approval = Onayınız bekleniyor
session-waiting-approval-note = Geçiş isteğini yanıtlayın. Henüz hiçbir şey değişmedi.
session-waiting-confirm = Ajan bekleniyor
session-skip-approved-pending = Onaylandı. Ajan değişikliği henüz doğrulamadı; doğruladığında burada görünür.
session-skip-unconfirmed = Ajan değişikliği henüz bildirmedi. Bildirdiğinde burada görünür.
session-skip-not-applied = Geçilmedi: istek reddedildi, süresi doldu veya geri çekildi. Ajan eskisi gibi sormaya devam eder.
session-forbidden-hint = bu makinede izinli değil
session-confirm-title = Ajan sormadan çalışsın mı?
session-confirm-body = { $name } modunda ajan komut çalıştırır ve dosyaları size sormadan değiştirir.
session-confirm-yes = Yine de geç
session-confirm-no = Sormaya devam etsin
grant-title-typing = { $agent } ajanına 1 saat boyunca yazmaya izin verilsin mi?
grant-title-prompt = { $agent } ajanına 1 saat boyunca mesaj göndermeye izin verilsin mi?
grant-title-stop = { $agent } ajanını 1 saat boyunca durdurmaya izin verilsin mi?
grant-title-interrupt = { $agent } ajanının sürmekte olan turunu 1 saat boyunca kesmeye izin verilsin mi?
grant-title-settings = { $agent } ajanının modunu, modelini veya ayarlarını 1 saat boyunca değiştirmeye izin verilsin mi?
grant-title-resize = { $agent } ajanının terminal boyutunu 1 saat boyunca değiştirmeye izin verilsin mi?
grant-title-watch = { $agent } ajanını 1 saat boyunca izlemeye izin verilsin mi?
grant-title-start = Bu projede 1 saat boyunca ajan başlatmaya izin verilsin mi?
grant-title-task = { $agent } ajanına 1 saat boyunca görev vermeye izin verilsin mi?
grant-title-other = Bu pencerenin 1 saat boyunca { $machine } üzerinde işlem yapmasına izin verilsin mi?
grant-explain = { $machine }, bu pencerenin bu işlemi yapmasına izin vermeden önce onayınızı ister. Verdiğiniz izin bu cihazdaki anahtarınızla imzalanır ve süresi dolunca kendiliğinden sona erer.
grant-allow = 1 saatliğine izin ver
grant-allowing = İzin veriliyor…
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
audit-action-plugin-add = Bir eklenti kurdu
audit-action-plugin-remove = Bir eklentiyi kaldırdı
audit-action-plugin-grant = Bir eklentinin neleri görebileceğini seçti
audit-action-plugin-ungrant = Bir eklentiyi göstermeyi bıraktı
audit-action-share-item-publish = Bir skill'i veya MCP sunucusunu tüm ajanlarla paylaştı
audit-action-share-item-remove = Bir skill'i veya MCP sunucusunu paylaşmayı bıraktı
audit-action-plugin-catalog-on = Eklenti kataloğunu açtı
audit-action-plugin-catalog-off = Eklenti kataloğunu kapattı
audit-action-plugin-catalog-key = Eklenti kataloğunun anahtarını ayarladı
audit-action-plugin-catalog-update = Eklenti kataloğunu getirdi
audit-action-plugin-catalog-install = Katalogdan bir eklenti kurdu
audit-action-plugin-catalog-upgrade = Katalogdan bir eklentiyi yükseltti
audit-action-settings-set = Temalarını cihazlarında aynı tuttu
audit-action-pair-open = Yeni makinelerin katılmak istemesine izin verdi
audit-action-pair-close = Yeni makinelerin katılmak istemesini kapattı
audit-action-pair-status = Yeni makinelerin katılmak isteyip isteyemeyeceğine baktı
audit-action-pair-approve = Bir makinenin katılmasına izin verdi
audit-action-pair-deny = Bir makineyi geri çevirdi
audit-action-pair-request = Bir makine katılmak istedi
audit-action-pair-reveal = Bir makine katılma isteğine baktı
audit-action-pair-issued = Bir makineye sertifikası verildi
audit-action-pair-refused = Katılma istekleri geri çevrildi
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
start-directory-role = Rolünüz bu klasörü seçmenize izin vermiyor. Sahibine sorun.
start-role = Rolünüz burada ajan başlatmanıza izin vermiyor. Gerekirse sahibine sorun.
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
ide-tab-messages = Ekip sohbeti
ide-more-tabs = Diğer sekmeler
ide-more-tabs-needs = gizli bir sekme sizi bekliyor
ide-more-tabs-failed = gizli bir sekme hatayla bitti
ide-more-tabs-working = gizli bir sekme çalışıyor
theme-label = Renk düzeni
theme-chooser-open = Tema seç
theme-chooser-title = Tema seç
theme-chooser-hint = Her temayı önizleyin, ardından her renk düzeninde kullanılacak temayı seçin.
theme-chooser-scheme = Gösterilecek temalar
theme-chooser-current = Şu an: { $name }
theme-chooser-selected = Seçili
theme-chooser-done = Bitti
theme-system = Sistemi izle
theme-system-short = Sistem
ide-shared = Paylaşılan
network-hub = Agora'nız
network-build = Bu makinede çalışan sürüm
network-never = hiç
network-heard = { $when } duyuldu
network-last-seen = son görülme: { $when }
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
closed-clear-short = Temizle
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
interrupted-section-short = Durdu
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
resume-failed-role = Rolünüz bu ajanı yeniden başlatmanıza izin vermiyor. Sahibine sorun.

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
agent-does-not-ask-explain = Size önce sormadan çalışacak şekilde başlatıldı
agent-does-not-ask-session-neutral-explain = Size önce sormadan çalışır; sona erene veya geri alınana kadar sürer
agent-does-not-ask-session-explain = Onayınızla, sona erene veya siz geri alana kadar size önce sormadan çalışacak şekilde geçirildi
agent-always-allowed-short = Daha az sorar
# ADR-0094: a person runs an agent by hand in this Shell (found from the program in front of its terminal, never its screen).
provider-anthropic = Anthropic
provider-openai = OpenAI
provider-google = Google
provider-mistral = Mistral
provider-xai = xAI
provider-deepseek = DeepSeek
provider-alibaba = Alibaba
provider-moonshot = Moonshot
map-panel = Harita
map-label = { $machines } makine ve { $agents } ajanın haritası
map-hub = Hub
map-you = siz
map-machine-offline = { $name } (çevrim dışı)
map-agent = { $name }, { $state }
map-message = { $from } → { $to }: { $text }
map-task = Görev: { $text }
preview-panel = Önizleme
preview-unavailable = Bu pencereden önizleme kullanılamıyor.
preview-none = Açık önizleme yok. Makinelerinizden birinde bir ajanın uygulamasının dinlediği portu açın.
preview-open-title = Bir port aç
preview-machine = Makine
preview-port = Port
preview-for = Süre
preview-hours = { $hours ->
    [one] { $hours } saat
   *[other] { $hours } saat
}
preview-open = Aç
preview-entering = Önizleme açılıyor…
preview-which = Önizleme
preview-forward = { $machine } · port { $port }
preview-address = Adres
preview-reload = Yenile
preview-new-tab = Yeni sekmede aç
preview-size = Boyut
preview-size-full = Tam
preview-size-tablet = Tablet
preview-size-phone = Telefon
preview-until = { $time } saatine kadar
preview-close = Kapat
preview-suggestions = Önerilen
preview-suggested = { $agent }, uygulamasının { $port } portunda olduğunu söylüyor ({ $machine })
preview-suggested-machine = { $machine } üzerinde { $port } portunu bir şey dinliyor
preview-others = Başka yerden açılanlar
preview-other-device = başka bir cihazdan açıldı
preview-error-no-origin = Bu pencere hub'a bir vekil sunucu ya da başka bir port üzerinden ulaştı; bu yüzden önizlemenin kendi portuna ulaşamıyor. Hub'ı kendi adresi ve portuyla açın.
preview-error-node-port = O port makinenin kendisine ait (hub'ın ya da Agora'nın); asla önizlenmez.
preview-error-privileged = 1024'ün altındaki portlar yalnızca makinenin sahibi izin verirse önizlenir.
preview-error-not-listening = Makinede o portu dinleyen bir şey yok.
preview-error-port = O port önizlenemez.
preview-error-unknown = O önizleme sona erdi.
preview-error-limit = Makinede zaten 8 önizleme açık. Önce birini kapatın.
preview-error-forbidden = Bir önizlemeyi yalnızca onu açan kişi görebilir.
preview-error-untrusted = Makine bu cihaza güvenmiyor. Önce makineden güvenin.
preview-error-old = O makine ya da hub önizleme için fazla eski.
preview-error-offline = Makine çevrim dışı.
preview-error-other = Açılmadı. Biraz sonra yeniden deneyin.
agent-provider-title = Sağlayıcı: { $value }
fleet-model-filter = Şunda çalışan ajanları göster
fleet-model-all = Her model
fleet-model-providers = Sağlayıcılar
fleet-model-models = Modeller
fleet-model-count = { $name } ({ $count })
agent-model-title = Model: { $value }
agent-effort-title = Akıl yürütme düzeyi: { $value }
agent-effort-chip = düzey { $value }
agent-runs-inside = Shell içinde { $name }
agent-runs-inside-explain = Makinesi bu programı terminalin önünde buldu, terminalde görünenden değil. npm betiği olarak kurulan bir ajan Node olarak çalışır ve tanınmaz; Windows'ta arka planda başlatılan bir ajan, istem öndeyken de işaretli kalır.
agent-always-allowed-explain = Bu oturum için { $count } isteğine izin verdin. Sona erene kadar bunlar için bir daha sormaz.
agent-always-allowed-explain-kept = { $count } isteğinin yanıtını hatırlamasına izin verdin. Bunlar için bir daha sormaz; yanıtları ne kadar tutacağına kendisi karar verir.
agent-does-not-ask-cause = Nedeni: { $value }
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
layout-panel-agent = Ajan
layout-panel-empty = Boş bölme
layout-panel-unknown = Bilinmeyen panel “{ $id }”: bu pencere onu gösteremiyor.
layout-panel-plugin = Eklenti paneli
layout-panel-plugin-safe = Güvenli kipte eklentiler kapalı. Bu paneli görmek için güvenli kipten çıkın.
layout-panel-plugin-missing = Bu eklenti bu tarayıcıda eklenmemiş. Panelini burada görmek için Eklentiler'den ekleyin.
layout-drag = Başka bir bölmenin üzerine sürükleyin: kenara bırakmak yanına koyar, ortaya bırakmak sekme yapar
layout-empty-title = Bu bölmede ne gösterilsin
layout-nothing-selected = Burada görmek için listeden bir ajan seçin.
layout-gone = Bu ajan artık yok.
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

## Klavye: komutlar ve tuşları (ADR-0068 4. basamak, docs/ux/keybindings.md)
palette-keyboard = Klavye ayarları
keys-cmd-agents = Ajan listesini göster veya gizle
keys-cmd-tab = { $n }. sekmeye git
keys-cmd-side = Yan panel, { $n }. sıra
keys-cmd-side-named = Yan panel: { $name }
keys-cmd-preset = Yerleşim: { $name }
keys-cmd-maximize = Bölmeyi büyüt veya geri al
keys-title = Klavye
keys-desc = Her eylem komut paletinde bir komuttur. Ona istediğiniz tuşları verin; tuşlar bu tarayıcıda saklanır.
keys-changed-count = { $count } değişti
keys-search = Bir komut veya tuş bulun
keys-import = İçe aktar…
keys-export = Dışa aktar
keys-reset-all = Tümünü sıfırla
keys-unreadable = Bu tarayıcıda saklanan tuşlar okunamadı; varsayılan tuşlar kullanılıyor.
keys-unknown = { $count ->
    [one] Tuşlarınız bu pencerede olmayan bir komutu anıyor. Saklanıyor, burada bir şey yapmıyor:
   *[other] Tuşlarınız bu pencerede olmayan { $count } komutu anıyor. Saklanıyorlar, burada bir şey yapmıyorlar:
}
keys-list = Komutlar ve tuşları
keys-none-match = Eşleşen komut yok.
keys-fixed-title = Değişmeyen tuşlar
keys-fixed-safe-mode = Güvenli kip
keys-fixed-navigation = Pencerede gezinmek
keys-fixed-terminal = Terminalde kopyalama, bulma ve yapıştırma
keys-fixed-editing = Metin düzenleme
keys-terminal-note = Klavye bir terminaldeyken tuşlarınız terminaldeki programa gider. Pencereye yalnızca programın hiç alamayacağı tuşlar ulaşır: ⌘ tuşları ve bir harfle Ctrl+Shift (Ctrl+Shift+K, terminaldeyken komut paletini açar).
keys-context-anywhere = Her yerde
keys-context-window = Pencerede
keys-context-pane = Odaktaki bölme
keys-context-tabs = Odaktaki sekmeler
keys-context-side = Yan panel
keys-changed = Değişti
keys-press = Tuşlara basın…
keys-press-second = { $key }, ardından ikinci bir tuş ya da bekleyin
keys-no-keys = Tuş yok
keys-fixed = Sabit
keys-change = Değiştir
keys-change-named = { $name } komutunun tuşlarını değiştir
keys-recording = Dinleniyor…
keys-clear = Temizle
keys-clear-named = { $name } komutunun tuşlarını temizle
keys-reset = Sıfırla
keys-reset-named = { $name } komutuna varsayılan tuşlarını geri ver
keys-conflict-same = { $key } aynı zamanda “{ $name }” komutunun tuşu.
keys-conflict-prefix = { $key } ile “{ $name }” aynı ilk tuşu paylaşıyor; ikisinden biri çalışamaz.
keys-refused-fixed = { $key } olduğu gibi kalır ({ $reason }).
keys-reset-title = Tüm tuşlar sıfırlansın mı?
keys-reset-body = Her komut varsayılan tuşlarına döner ve bu tarayıcıda saklanan değişiklikler silinir.
keys-reset-keep = Tuşlarım kalsın
keys-reset-confirm = Tümünü sıfırla
keys-import-title = Tuşları içe aktar
keys-import-paste = Bir tuş dosyası yapıştırın (JSON)
keys-import-file = Ya da bir dosya seçin
keys-import-refused = Bu dosya içe aktarılmadı:
keys-error-json = Bu geçerli bir JSON değil.
keys-error-too-big = Bu dosya çok büyük (en çok 64 KB).
keys-error-shape = Bu, bu pencerenin okuyabildiği bir tuş dosyası değil.
keys-error-newer = Bu dosya daha yeni bir sürümle yapılmış. Kullanmak için güncelleyin.
keys-error-too-many = Bu dosyada çok fazla komut var (en çok 200).
keys-error-field = Bilinmeyen alan “{ $name }”.
keys-error-command = “{ $command }” bir komut adı değil.
keys-error-request = “{ $command }”: bir istek kendi kartında yanıtlanır, asla bir tuşla değil.
keys-error-fixed-command = “{ $command }” tuşlarını korur; değiştirilemezler.
keys-error-twice = “{ $command }” iki kez yazılmış.
keys-error-too-many-keys = “{ $command }” için çok fazla tuş var (en çok 4).
keys-error-key = { $command } için “{ $key }” bu pencerenin tanıdığı bir tuş değil.
keys-error-fixed-key = { $command } için { $key } olduğu gibi kalır ({ $reason }).
keys-error-more = …ve { $count } tane daha.

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
theme-starter = başlangıç teması
theme-starter-hint = Palet: { $project } ({ $licence } lisansı). Başlangıç temaları değiştirilemez. Özelleştir, değiştirebileceğiniz bir kopya oluşturur.
theme-group-built-in = Yerleşik
theme-group-starter = Başlangıç temaları
theme-group-yours = Temalarınız
theme-customize = Özelleştir
theme-copy-name = { $name } (benim)
theme-colours = Renkler
theme-shape = Yazı ve biçim
theme-readability = Okunabilirlik
theme-incomplete = Bazı değerler henüz geçerli değil; pencere son geçerli değerleri korur.
theme-contrast-ok = Tüm yazılar ve denetimler kolay görünür (WCAG AA).
theme-contrast-not-applied = Uygulanmadı: bazı yazılar zor okunur.
theme-contrast-warning = Uygulandı, ancak bazı yerlerde yazılar veya denetimler zor görünür:
theme-contrast-also = Bazı yerlerde ayrıca zor görünür:
theme-import-warning = Eklendi. Gösterildiğinde bazı yerlerde yazılar veya denetimler zor görünür:
theme-px = { $size } px
theme-import = İçe aktar…
theme-export = Dışa aktar
theme-delete = Temayı sil
theme-reset = Varsayılanlara dön
theme-import-hint = Temalar bu tarayıcıda saklanır. Başka bir tarayıcıda kullanmak için dışa aktarın; Varsayılanlara dön, Açık ve Koyu'yu geri getirir ve temalarınızı korur.
theme-import-other = Başka bir uygulamadan içe aktar…
theme-import-title = Başka bir uygulamadan içe aktar
theme-import-intro = iTerm2 (.itermcolors), Alacritty (.toml veya .yml), Ghostty (tema dosyası), Windows Terminal (renk şeması) ve VS Code (renk teması) renklerini okur. Temayı ekle'yi seçene kadar hiçbir şey eklenmez.
theme-import-from = { $format } kaynağından: { $name }
theme-import-palette = Terminalin renkleri
theme-import-notes = Aktarırken değişenler
theme-import-refused = Bu tema eklenemez:
theme-import-warnings = Eklenebilir, ancak bazı yerlerde yazılar veya denetimler zor görünür:
theme-import-add = Temayı ekle
theme-import-cancel = Vazgeç
theme-import-default-name = İçe aktarılan tema
theme-import-and-more = { $list } ve { $count } tane daha
theme-import-format-iterm2 = iTerm2
theme-import-format-alacritty = Alacritty
theme-import-format-ghostty = Ghostty
theme-import-format-windows-terminal = Windows Terminal
theme-import-format-vscode = VS Code
theme-import-error-too-large = Bu dosya { $max } KiB'tan büyük; bir renk dosyası çok daha küçüktür.
theme-import-error-unknown-format = Bu, iTerm2, Alacritty, Ghostty, Windows Terminal veya VS Code renk dosyası değil.
theme-import-error-unreadable = Bu dosya bir renk dosyasına benziyor ama okunamıyor.
theme-import-error-entities = Bu dosya kendi XML varlıklarını tanımlıyor; bir renk dosyasının buna hiç ihtiyacı olmaz, bu yüzden okunmadı.
theme-import-error-no-colours = Bu dosyada terminal rengi bulunamadı.
theme-import-note-scheme = { $scheme ->
    [light] Arka planı açık, bu yüzden açık temanız olur. Koyu temanız olduğu gibi kalır.
   *[dark] Arka planı koyu, bu yüzden koyu temanız olur. Açık temanız olduğu gibi kalır.
  }
theme-import-note-derived = Pencerenin renkleri (arka planlar, yazı, vurgu ve durumlar) terminalin renklerinden yapıldı; her biri kolay okunacak şekilde denetlendi.
theme-import-note-derived-kept = Bu renklerden kolay okunan pencere renkleri yapılamadı; pencere Agora'nın kendi renklerini korur, terminal dosyadakileri alır.
theme-import-note-missing = Dosyada yok, Agora'nın kendi renkleri kullanılır: { $keys }.
theme-import-note-ignored = Alınmadı, temada bunların yeri yok: { $keys }.
theme-import-note-unreadable = Bu renkler okunamadı, Agora'nın kendi renkleri kullanılır: { $keys }.
theme-import-note-alpha = Dosyada saydam, bu yüzden arka planın üzerine karıştırıldı: { $keys }.
theme-import-note-background-alpha = Dosyada arka plan saydam; pencere onu düz çizer.
theme-import-note-selection-alpha = Seçimin saydamlığı alınmadı: Agora seçimi zaten saydam çizer.
theme-import-note-colour-space = { $space ->
    [p3] Renkler Display P3'ten sRGB'ye dönüştürüldü.
   *[other] Bazı renkler sRGB veya Display P3 dışında bir renk uzayında; sRGB olarak okundu, biraz farklı görünebilir.
  }
theme-import-note-variant = { $used ->
    [light] Dosyada ayrı açık ve koyu renkler var; açık olanlar kullanıldı.
   *[dark] Dosyada ayrı açık ve koyu renkler var; koyu olanlar kullanıldı.
  }
theme-import-note-first-scheme = Dosyada { $count } renk şeması var; ilki içe aktarıldı.
theme-import-note-window-colours = VS Code'un { $count } pencere rengi alınmadı: pencerenin renkleri terminalin renklerinden yapılır.
theme-import-note-include = Bu tema başka bir dosyanın üzerine kurulu (include); o dosya okunmadı, yalnızca bu dosyadaki renkler kullanıldı.
theme-import-note-fallback = Dosyada { $key } yok, bu yüzden { $from } değerinden alındı.
theme-import-note-syntax = { $count } kod rengi temanın simge renklerinden alındı.
theme-catalog = Katalog
theme-catalog-hint = Agora'nızın sahibinin eklediği topluluk temaları. Kurmak birini temalarınıza kopyalar; o tema yalnızca yeni sürümünü seçtiğinizde değişir.
theme-catalog-empty = Katalogda henüz tema yok.
theme-catalog-by = { $author } · { $licence }
theme-catalog-version = Sürüm { $version }
theme-catalog-installed = Kurulu
theme-catalog-newer = Sürüm { $version } çıktı
theme-catalog-preview = Önizle
theme-catalog-hide = Önizlemeyi gizle
theme-catalog-install = Kur
theme-catalog-update = { $version }. sürüme güncelle
theme-catalog-refused = Bu tema kullanılamıyor:
theme-catalog-gone = Bu tema artık katalogda değil.
theme-catalog-off = Katalog kapatıldı.
theme-catalog-unavailable = Tema getirilemedi. Yeniden deneyin.
theme-sync-label = Temalarım tüm cihazlarımda aynı olsun
theme-sync-syncing = Temalarınız, paketleriniz ve tema seçiminiz oturum açtığınız her cihazda aynı olur.
theme-sync-synced = Temalarınız, paketleriniz ve tema seçiminiz oturum açtığınız her cihazda aynı olur.
theme-sync-off = Temalarınız bu tarayıcıda kalır. Diğer cihazlarınızda da olmaları için bunu açın.
theme-sync-safe = Güvenli mod: cihazlarınızda hiçbir şey eşitlenmez.
theme-sync-local = Temalarınız bu tarayıcıda kalır. Diğer cihazlarınızda da olmaları için Agora'nızda oturum açın.
theme-sync-older = Agora'nız henüz temaları saklayamıyor, bu yüzden bu tarayıcıda kalıyorlar.
theme-sync-too-large = Temalarınız ve paketleriniz Agora'nızda saklanamayacak kadar büyük (en fazla { $max } KB), bu yüzden bu tarayıcıda kalıyorlar. Bir temayı silin ya da bir paketi unutun.
theme-sync-newer = Agora'nızda saklanan temalar daha yeni bir sürümle kaydedilmiş. Kullanmak için bu pencereyi güncelleyin; o zamana kadar sizinkiler bu tarayıcıda kalır.
theme-sync-error = Agora'nıza şu an ulaşılamadı. Temalarınız burada duruyor ve bir sonraki değişikliğinizde kaydedilecek.
theme-sync-paused = Tema eşitleme duraklatıldı: bu tarayıcının başka bir sekmesinde başka biri oturum açtı. Eşitlemek için burada yeniden oturum açın.
theme-synced-title = Paketleriniz ve cihazlarınız
theme-synced-hint = Burada ve diğer cihazlarınızda içe aktardığınız paketler. Birini açmak içinde ne olduğunu gösterir; parçalarını seçene kadar hiçbir şey değişmez.
theme-synced-packs = Paketleriniz
theme-synced-open = Aç…
theme-synced-forget = Unut
theme-synced-remove = Cihazlarımdan kaldır
theme-synced-waiting = { $count ->
    [one] Diğer cihazlarınızdan 1 tema buraya sığmıyor: en fazla { $max } tema tutabilirsiniz. Yer açmak için kendi temalarınızdan birini silin.
   *[other] Diğer cihazlarınızdan { $count } tema buraya sığmıyor: en fazla { $max } tema tutabilirsiniz. Yer açmak için kendi temalarınızdan birini silin.
}
theme-synced-waiting-packs = { $count ->
    [one] Diğer cihazlarınızdan 1 paket burada tutulmuyor: en fazla { $max } paket tutulur. Yer açmak için birini unutun.
   *[other] Diğer cihazlarınızdan { $count } paket burada tutulmuyor: en fazla { $max } paket tutulur. Yer açmak için birini unutun.
}
theme-synced-invalid-theme = Diğer cihazlarınızdan gelen "{ $name }" teması burada kullanılamıyor:
theme-synced-invalid-theme-unnamed = Diğer cihazlarınızdan gelen bir tema burada kullanılamıyor:
theme-synced-invalid-pack = Diğer cihazlarınızdan gelen "{ $name }" paketi burada kullanılamıyor:
theme-synced-invalid-pack-unnamed = Diğer cihazlarınızdan gelen bir paket burada kullanılamıyor:
pack-sync-themes = { $count ->
    [one] 1 tema
   *[other] { $count } tema
}
pack-sync-layouts = { $count ->
    [one] 1 kayıtlı yerleşim
   *[other] { $count } kayıtlı yerleşim
}
pack-sync-keys = { $count ->
    [one] 1 komutun tuşları
   *[other] { $count } komutun tuşları
}
pack-import = Paket içe aktar…
theme-import-is-pack = Bu bir tema paketi. İçinde ne olduğunu görmek ve neyi kullanacağınızı seçmek için Paket içe aktar…’ı kullanın.
pack-export = Paket olarak dışa aktar…
pack-export-title = Paket olarak dışa aktar
pack-export-hint = Paket; temaları, kayıtlı yerleşimleri ve tuşları paylaşmak ya da başka bir tarayıcıda kullanmak için tek bir dosyadır. İçine neyin gireceğini seçin.
pack-name = Paket adı
pack-name-default = Paketim
pack-themes = Temalar
pack-layouts = Kayıtlı yerleşimler
pack-keys = Tuşlar
pack-panes = { $count ->
    [one] 1 bölme
   *[other] { $count } bölme
}
pack-export-no-layouts = Henüz kayıtlı yerleşim yok. Yerleşim menüsünden bir tane kaydedin.
pack-export-unbound = Ajanlarınızdan birini gösteren bölme onsuz kaydedilir: paket yalnızca panelleri adlandırır.
pack-keys-own = Tuşlarınız
pack-keys-none = Varsayılan tuşları kullanıyorsunuz.
pack-keys-count = { $count ->
    [one] Kendi tuşları olan 1 komut
   *[other] Kendi tuşları olan { $count } komut
}
pack-export-do = Dışa aktar
pack-import-title = Paket içe aktar
pack-import-hint = Bir şey değişmeden önce paketin içinde ne olduğunu görür ve neyi kullanacağınızı seçersiniz.
pack-import-paste = Bir paket yapıştırın (JSON)
pack-import-file = Ya da bir dosya seçin
pack-import-refused = Bu paket kullanılamaz:
pack-preview-do = İçinde ne var, gör
pack-preview-title = “{ $name }” paketi
pack-preview-hint = Kullanılacakları işaretleyin. Siz işaretleyene kadar hiçbir şey değişmez. Temalar ve yerleşimler sizinkilere eklenir; tuşlar, neyi değiştireceklerini gördükten sonra birleştirilir.
pack-theme-warnings = { $count ->
    [one] zor görünen 1 şey
   *[other] zor görünen { $count } şey
}
pack-themes-room = { $count ->
    [one] 1 tema daha
   *[other] { $count } tema daha
} için yeriniz var. Kendi temalarınızdan birini silin ya da daha azını işaretleyin.
pack-layout-plugin = { $panels } için bir eklenti gerekir. Paket hiçbir zaman eklenti kurmaz: siz kurup izin verene kadar bölme gösterilemediğini söyler.
pack-layout-unknown = Bu pencere { $panels } gösteremez: bölme bunu söyler.
pack-keys-theirs = Paketin tuşları
pack-keys-unknown = { $count ->
    [one] Bu pencerede olmayan bir komutu adlandırıyor. Saklanır ve burada bir şey yapmaz.
   *[other] Bu pencerede olmayan { $count } komutu adlandırıyor. Saklanırlar ve burada bir şey yapmazlar.
}
pack-back = Geri
pack-next = İleri
pack-apply = İşaretlileri kullan
pack-close = Kapat
pack-keys-confirm-title = Paketin tuşları birleştirilsin mi?
pack-keys-confirm-hint = { $count ->
    [one] Paket 1 komutun tuşlarını belirliyor. Diğer tuşlarınız olduğu gibi kalır.
   *[other] Paket { $count } komutun tuşlarını belirliyor. Diğer tuşlarınız olduğu gibi kalır.
}
pack-keys-no-conflicts = Kendi belirlediğiniz hiçbir şey değişmez ve hiçbir tuş iki iş yapmaz.
pack-keys-replaced = Bunların yerini aldığı tuşlarınız
pack-keys-replaced-row = { $name }: { $mine } yerine { $theirs }
pack-keys-clashes = İki iş yapacak tuşlar
pack-keys-clash-same = { $key } hem { $a } hem { $b } olur.
pack-keys-clash-prefix = { $key } hem { $a } hem { $b } tuşunu başlatır.
pack-keys-merge-do = Tuşları birleştir ve işaretlileri kullan
pack-layout-copy = { $name } ({ $n })
pack-done-themes = { $count ->
    [one] 1 tema sizinkilere eklendi. İstediğinizde yukarıdan seçin.
   *[other] { $count } tema sizinkilere eklendi. İstediğinizde yukarıdan birini seçin.
}
pack-done-layouts = { $count ->
    [one] 1 yerleşim kaydedildi: { $names }. Yerleşim menüsünde bulabilirsiniz.
   *[other] { $count } yerleşim kaydedildi: { $names }. Yerleşim menüsünde bulabilirsiniz.
}
pack-done-keys = { $count ->
    [one] 1 komutun tuşları birleştirildi.
   *[other] { $count } komutun tuşları birleştirildi.
}
pack-error-json = Bu geçerli bir JSON değil.
pack-error-invalid = Bu bir paket değil.
pack-error-newer = Bu paket daha yeni bir sürümle yapılmış. Kullanmak için güncelleyin.
pack-error-empty = Bu pakette kullanılacak bir şey yok.
pack-error-name = Paketin 1 ile { $max } karakter arasında, “;”, “<” ya da “url(” içermeyen bir adı olmalı.
pack-error-too-large = Bu paket { $max } KB'tan büyük.
pack-error-unknown-field = “{ $field }” bir paketin parçası değil.
pack-error-part = “{ $part }”, paketin onu tuttuğu biçimde değil.
pack-error-too-many-themes = Bir pakette en çok { $max } tema olabilir.
pack-error-too-many-layouts = Bir pakette en çok { $max } yerleşim olabilir.
pack-error-theme-format = { $n }. tema güncel biçimde bir tema değil.
pack-in-theme = { $n }. tema: { $text }
pack-in-layout = { $n }. yerleşim: { $text }
pack-in-keys = Tuşlar: { $text }
pack-error-layout-entry = { $n }. yerleşim adlandırılmış bir yerleşim değil.
pack-error-layout-name = { $n }. yerleşimin 1 ile 60 karakter arasında, “;”, “<” ya da “url(” içermeyen bir adı olmalı.
pack-error-layout-twice = { $n }. yerleşim: “{ $name }” adı iki kez kullanılmış.
pack-error-layout-field = { $n }. yerleşim: “{ $field }” bir yerleşimin parçası değil.
pack-error-layout-panel = { $n }. yerleşim: “{ $panel }” bir panel adı değil.
pack-error-layout-agent = { $n }. yerleşim birinin ajanlarından birini gösteriyor. Paketteki yerleşim yalnızca panelleri adlandırır.
pack-error-layouts-full = En çok { $max } kayıtlı yerleşim tutabilirsiniz. Bazılarını silin ya da daha azını işaretleyin.
pack-error-keys-merge = Tuşlar birleştirilemiyor: { $text }
pack-error-more = …ve { $count } tane daha.
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
theme-tint-accent = Vurgu arka planı
theme-tint-working = Çalışıyor arka planı
theme-tint-needs = Sizi bekliyor arka planı
theme-tint-finished = Bitti arka planı
theme-tint-danger = Hata arka planı
theme-tint-idle = Boşta arka planı
theme-colour-danger = Kırmızı (hatalar)
theme-colour-on-danger = Kırmızı üzerindeki yazı
theme-colour-surface-raised = Seçili seçenek arka planı
theme-chip-accent = Vurgu simgesi arka planı
theme-chip-working = Çalışıyor simgesi arka planı
theme-chip-needs = Sizi bekliyor simgesi arka planı
theme-chip-finished = Bitti simgesi arka planı
theme-chip-danger = Hata simgesi arka planı
theme-chip-idle = Boşta simgesi arka planı
theme-avatar = { $colour } ajan rozeti
theme-agent-colour-amber = Kehribar
theme-agent-colour-teal = Turkuaz
theme-agent-colour-violet = Mor
theme-agent-colour-rose = Gül
theme-agent-colour-slate = Arduvaz
theme-agent-colour-sky = Gök mavisi
theme-agent-colour-mint = Nane
theme-control-segmented = Seçili seçenek halkası
theme-control-risk-segmented = Riskli seçili seçenek halkası
theme-control-switch-off = Anahtar (kapalı)
theme-control-switch-on = Anahtar (açık)
theme-control-risk-switch = Riskli anahtar (açık)
theme-control-selected-card = Seçili kart halkası
theme-control-pressed-count = Seçili sayaç halkası
theme-control-primary-button = Ana düğme
theme-control-danger-button = Kırmızı düğme
theme-control-focus-ring = Odak halkası
theme-control-icon = Simge
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
theme-error-control = { $control }: { $bg } üzerinde { $fg } { $ratio }:1; görünür olması için en az { $min }:1 gerekir.
theme-error-full = { $max } temanız var; önce birini silin.
# Tema biçimi 2 (docs/ux/themes.md, ADR-0113): bir dosyadaki sorunlar, bulundukları yerle ("dark.regions.sidebar.text").
theme-error-seen = { $bg } üzerinde { $fg } { $ratio }:1; görünür olması için en az { $min }:1 gerekir.
theme-error-no-look = Temanın bir "dark" (koyu) veya "light" (açık) bölümü ya da başlangıç olarak bir yerleşik teması ("inherits") olmalı.
theme-error-moved-token = "{ $field }": "{ $token }", "{ $to }" içine yazılır.
theme-error-field-color = "{ $field }", #1a2b3c gibi bir renk olmalı.
theme-error-field-size = "{ $field }", { $min } ile { $max } arasında tam sayı piksel olmalı.
theme-error-field-list = "{ $field }" şunlardan biri olmalı: { $values }.
theme-error-field-font = "{ $field }", virgülle ayrılmış yazı tipi adları olmalı.
theme-error-field-line-height = "{ $field }", "normal" ya da { $min } ile { $max } arasında bir sayı olmalı.
theme-error-field-switch = "{ $field }", true ya da false olmalı.
theme-error-field-group = "{ $field }", adlandırılmış değerlerden oluşan bir grup olmalı.
theme-error-unsafe = "{ $field }" bir temada bulunamayacak bir metin içeriyor ("url(", ";" veya "<" gibi): tema yalnızca renk, boyut, listeden sözcük ve yazı tipi adı içerir.
theme-error-floor = "{ $field }" ayarlanamaz. { $reason }
theme-error-text-floor = "{ $field }": hiçbir yazı 12 pikselden küçük değildir, bu yüzden yazı boyutu en az { $min } px olur.
theme-warning-text-lifted = { $from } yazı boyutu bazı yazıları 12 pikselden küçük yapardı: { $to } olarak gösteriliyor.
theme-error-inherits = "{ $id }" başlangıç alınabilecek bir yerleşik tema değil.
theme-error-too-large = Bu dosya { $max } KiB'tan büyük; bir tema çok daha küçüktür.
theme-error-more = …ve { $count } sorun daha.
theme-issue-in-scheme = { $scheme }: { $text }
theme-floor = Hep aynı kalanlar
theme-floor-hint = Hiçbir tema bunları değiştiremez; böylece pencere sizi bekleyen şeyi asla gizlemez ya da taklit etmez.
theme-floor-row-danger = Kırmızı (hatalar ve tehlike)
theme-floor-row-diff-removed = Silinen satırlar
theme-floor-row-approval-card = Ajanlardan gelen istekler
theme-floor-row-focus-ring = Odak halkası
theme-floor-row-needs-you = "Sizi bekliyor"
theme-floor-row-plugin-marker = Eklenti işareti
theme-floor-danger = Hatalar ve tehlike için kırmızı hep aynıdır; gizlenemez ya da kopyalanamaz.
theme-floor-diff-removed = Silinen satırlar hep aynı kırmızıdır.
theme-floor-approval-card = Bir ajandan gelen istek hep aynı görünür; hiçbir tema sorduğunu gizleyemez ya da taklit edemez.
theme-floor-focus-ring = Odak halkası klavyenin nerede olduğunu hep gösterir.
theme-floor-needs-you = "Sizi bekliyor" şeklini, sözcüğünü ve yerini hep korur.
theme-floor-plugin-marker = Bir eklentinin eklediği her şey hep işaretlidir.
theme-locked = Kilitli
theme-scheme-editing = { $scheme } renkler gösteriliyor. Diğerlerini düzenlemek için renk düzenini değiştirin.
theme-scheme-add = { $scheme } renkleri ekle
theme-scheme-add-hint = Bu temanın yalnızca { $scheme } renkleri var. İkisi için de kullanmak için diğerlerini ekleyin.
theme-regions = Pencerenin bölümleri
theme-regions-hint = Boş bir alan yukarıdaki renkleri izler.
theme-follows = Temayı izler
theme-clear = Temanın rengini kullan
theme-region-sidebar = Ajan listesi
theme-region-tab-bar = Sekmeler
theme-region-pane-bar = Bölme çubukları
theme-region-status-line = Durum satırı
theme-region-cards = Kartlar
theme-region-inputs = Metin alanları
theme-region-buttons = Düğmeler
theme-region-agent-entries = Ajan yanıtları
theme-region-person-entries = Sizin iletileriniz
theme-region-code-blocks = Kod blokları
theme-region-part-surface = { $region }: arka plan
theme-region-part-text = { $region }: yazı
theme-region-part-line = { $region }: çizgiler
theme-part-surface = Arka plan
theme-part-text = Yazı
theme-part-line = Çizgiler
theme-terminal = Terminal
theme-terminal-hint = Boş bir alan pencerenin kendi terminal renklerini kullanır.
theme-terminal-background = Terminal arka planı
theme-terminal-foreground = Terminal yazısı
theme-terminal-cursor = İmleç
theme-terminal-selection = Seçim
theme-terminal-black = Siyah
theme-terminal-red = Kırmızı
theme-terminal-green = Yeşil
theme-terminal-yellow = Sarı
theme-terminal-blue = Mavi
theme-terminal-magenta = Eflatun
theme-terminal-cyan = Camgöbeği
theme-terminal-white = Beyaz
theme-terminal-bright-black = Parlak siyah
theme-terminal-bright-red = Parlak kırmızı
theme-terminal-bright-green = Parlak yeşil
theme-terminal-bright-yellow = Parlak sarı
theme-terminal-bright-blue = Parlak mavi
theme-terminal-bright-magenta = Parlak eflatun
theme-terminal-bright-cyan = Parlak camgöbeği
theme-terminal-bright-white = Parlak beyaz
theme-syntax = Kod renkleri
theme-syntax-hint = Boş bir alan temanın kendi renklerinden karıştırılır.
theme-syntax-keyword = Anahtar sözcükler
theme-syntax-string = Tırnak içindeki metin
theme-syntax-comment = Yorumlar
theme-syntax-number = Sayılar
theme-syntax-function = İşlev adları
theme-syntax-type = Türler
theme-syntax-attr = Adlar ve öznitelikler
theme-syntax-meta = Diğer işaretler
theme-chrome = Pencere
theme-chrome-pane-borders = Bölme kenarlıkları
theme-chrome-pane-borders-auto = Bölmeler arasında
theme-chrome-pane-borders-always = Her bölmenin çevresinde
theme-chrome-pane-borders-off = Yok
theme-chrome-pane-gap = Bölmeler arası boşluk
theme-chrome-line-width = Çizgi kalınlığı
theme-chrome-tab-bar = Sekme çubuğu
theme-chrome-tab-bar-top = Üstte
theme-chrome-tab-bar-bottom = Altta
theme-chrome-tab-style = Sekme biçimi
theme-chrome-tab-style-underline = Çizgi
theme-chrome-tab-style-filled = Dolu
theme-chrome-tab-style-pill = Yuvarlak
theme-chrome-density = Sıklık
theme-chrome-density-compact = Sık
theme-chrome-density-normal = Normal
theme-chrome-density-roomy = Ferah
theme-chrome-state-display = Listelerde ve sekmelerde ajan durumu
theme-chrome-state-display-symbol = Simge
theme-chrome-state-display-dot = Nokta
theme-chrome-state-display-word = Sözcük
theme-chrome-state-display-hint = Sözcük her zaman ipucunda yazar ve sesli okunur; "Sizi bekliyor" kendi simgesini korur.
theme-chrome-icons = Simgeler
theme-chrome-icons-lines = Çizgi
theme-chrome-icons-hint = Şimdilik tek bir simge takımı var.
theme-chrome-shadow = Gölgeler
theme-chrome-shadow-none = Yok
theme-chrome-shadow-soft = Yumuşak
theme-chrome-shadow-strong = Güçlü
theme-chrome-motion = Hareket
theme-chrome-motion-system = Sistemi izle
theme-chrome-motion-reduced = Azaltılmış
theme-chrome-motion-off = Kapalı
theme-text-weight = Yazı kalınlığı
theme-text-weight-300 = İnce
theme-text-weight-400 = Normal
theme-text-weight-500 = Orta
theme-text-line-height = Satır yüksekliği
theme-text-line-height-normal = Normal
theme-text-ligatures = Kodda birleşik harfler (bitişik harfler)
theme-text-ligatures-hint = Terminallerde değil.
theme-preview = Önizleme
theme-preview-agent = Testler artık geçiyor.
theme-preview-person = Testleri yeniden çalıştır.
theme-preview-field = Buraya yazın
theme-preview-button = Düğme
theme-preview-card = Bir kart
theme-preview-status = 2 makine çevrimiçi
theme-preview-tab = Sekme
theme-preview-pane = Bölme
theme-preview-sample-command = $ npm test
theme-preview-sample-pass = ✓ 41 geçti
theme-preview-sample-fail = ✗ 1 başarısız
theme-preview-sample-note = (uyarı) 2 atlandı
palette-theme-editor = Temayı düzenle
palette-theme = Tema: { $name }
palette-open-machine = Makine: { $name }
palette-open-project = Proje: { $name }
palette-settings = Ayarlar
device-appearance = Görünüm
device-appearance-intro = Bu tarayıcı { $name } temasını gösteriyor.

## ADR-0067 2. basamak: telefon boyutunda pencere ve terminal tuş çubuğu.
phone-nav = Git
phone-search = Ara
phone-start = Başlat
phone-machines = Makineler
phone-more = Diğer
phone-back = Listeye dön
phone-offline = çevrim dışı
phone-no-agents = Bu makinede ajan yok.
phone-needs = Sizi bekleyenler · { $count }
phone-answer-below = Yanıtı ekranın altındaki düğmelerle verin.
phone-ask-fold = Bu isteği tek satıra indir
phone-next = Sıradaki · { $count }
phone-untrusted-title = Bu cihaz henüz güvenilir değil
phone-untrusted-machines = { $count ->
    [one] { $machines } bu cihaza henüz güvenmiyor; buradan verilen yanıtı reddeder.
   *[other] { $machines } bu cihaza henüz güvenmiyor; buradan verilen yanıtları reddederler.
}
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
forget-machines-cancel = Kalsın
forget-machines-confirm = { $count ->
    [one] Unut
   *[other] { $count } makineyi unut
}
forget-machine-reason-forbidden = Makineleri yalnızca sahibi unutabilir.
forget-machine-reason-online = Yeniden bağlandı, bu yüzden unutulmadı.
forget-machine-reason-gone = Agora'nız bu makineyi artık tanımıyor.
forget-machine-reason-other = Unutulamadı. Yeniden deneyin.

# My machines as cards (the redesign's PR 9): the page's header, the steps to add a machine, the strip of counts, a machine's card.
machines-intro = Agora'nıza bağlı her makine. Bir makine uzaktayken ajanları orada çalışmaya devam eder.
machines-add = Makine ekle
machines-add-step-allow = Yeni bir makinenin katılmasına izin verin. 10 dakika açık kalır.
machines-add-step-run = Yeni makinede şu komutu çalıştırın:
machines-add-command = agora join
machines-add-step-compare = Ardından iki ekranda da bir numara görünür. Yalnızca ikisi aynıysa burada onaylayın.
machines-add-hint = Yeni makinelere izin verin, o makinede agora join çalıştırın, sonra numarayı burada karşılaştırın.
machines-stat-of = / { $total }
machines-stat-online = { $total ->
    [one] makine çevrimiçi
   *[other] makine çevrimiçi
}
machines-stat-agents = { $count ->
    [one] ajan çalışıyor
   *[other] ajan çalışıyor
}
machines-stat-needs = { $count ->
    [one] sizi bekliyor
   *[other] sizi bekliyor
}
machines-stat-memory = Agora'nın kullandığı bellek
machine-memory = Bu makinede Agora
machine-memory-share = Agora'nın tüm makinelerinizde kullandığı belleğin %{ $percent } kadarı
machine-start-here = Burada bir ajan başlat
machine-new-terminal = Yeni terminal
machine-more = { $name } için diğer işlemler
machine-copy-id = Makine kimliğini kopyala
machine-agent-settings = Ajan ayarları
machine-away-agents = { $count ->
    [one] Bir ajan çalışıyordu
   *[other] { $count } ajan çalışıyordu
}
machine-away-explain = { $name } üzerinde çalışmaya devam ederler. Yaptıkları, makine yeniden bağlandığında burada görünür.
machine-not-back = Geri gelmeyecek mi?
forget-machine-explain = Makine ve biten ajanları her pencereden kalkar. Geçmişi ve kullanımı kalır. Yeniden bağlanırsa geri gelir.
forget-machine-refuse = Yeniden bağlanırsa reddet
forget-machine-refuse-hint = Yeni bir katılım kodu gerekir.
forget-machines-refuse-title = Yeniden bağlanırlarsa reddet
forget-machines-refuse-hint = Her biri yeni bir katılım kodu ister.
forget-machine-confirm-one = { $name } makinesini unut

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
fleet-stat-machines =
    { $total ->
        [0] Çevrimiçi makine
       *[other] { $total } makineden çevrimiçi olan
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
fleet-guest-title = Henüz size ait bir proje yok
fleet-guest-text = Yalnızca size erişim verilen projeleri görürsünüz. Sahibinden sizi bir projeye eklemesini isteyin; eklediği anda burada görünür.
fleet-firstrun-title = Makineleriniz hazır. İlk ajanı başlatın.
fleet-firstrun-text = Bir makine ve klasör seçin. Ajan burada görünür ve size gerektiğinde haber verir.
fleet-firstrun-cli = Ya da o makinedeki bir terminalden
fleet-firstrun-offline = Şu an tüm makineler çevrimdışı. Birinde Agora'yı başlatın, burada görünsün.
fleet-watch-title = Henüz çalışan bir şey yok
fleet-watch-text = Burada izleyebilirsiniz. Üyelerin ya da sahibin başlattığı ajanlar, neye ihtiyaç duydukları ile birlikte burada görünür.
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
attention-ask-review = Yanıtlamadan önce bunun tamamı okunmalı. Karar vermek için incele.
attention-skip = { $name } sormadan çalışmaya geçirilmek istendi. Yanıtlamadan önce İncele isteğin tamamını gösterir.
attention-start-read = Yanıtlamadan önce İncele klasörünü, istemini ve ayarlarını tam olarak gösterir.
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
attention-mode-modal = Pencerenin üstünde bir kart
attention-mode-banner = Yalnızca küçük bir şerit
attention-mode-off = Ek bir şey yok
attention-mode-hint = Kart yalnızca bu pencere odaktayken ve yazmıyorken açılır; aksi halde bir şerit onu gösterir. Senin yerine asla yanıt vermez.
attention-notify-title = Arka planda bildirimler
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
# A request card in the side panel or a pane, after its answer went (it has no Review button: the card is the request).
approval-sent = Yanıt gönderildi.
approval-watching = Burada izleyebilirsiniz. İstekleri bir üye ya da sahibi yanıtlar.
attention-notify-names-warn = Bu adlar ajanlardan ve makinelerden gelir; bildirim kilitli ekranda görünebilir. Düz metin olmayan bir ad bildirime konmaz.

# Usage (no money: tokens, context and time)
ide-tab-cost = Kullanım
ide-tab-usage = Kullanım
usage-stat-tokens = Kullanılan token
usage-stat-agents = Ajan
usage-agents = { $count } ajan
usage-by-machine = Makineye göre
usage-by-project = Projeye göre
usage-context = Bağlam %{ $percent }
usage-limits-title = Plan limitleri
usage-limits-empty = Ajan bildirdiğinde plan limitleri burada görünür.

# Sohbet ajanının konuşması
chat-jump = En sona git
chat-jump-new = En sona git, { $count } yeni
chat-earlier-items = Öncekileri göster ({ $count })
chat-you = Siz
chat-agent = Ajan
chat-copy-reply = Yanıtı kopyala
chat-copy-message = Mesajı kopyala
chat-act-read = { $count } dosya okudu
chat-act-edit = { $count } dosyayı düzenledi
chat-act-run = { $count } komut çalıştırdı
chat-act-search = { $count } kez aradı
chat-act-fetch = { $count } sayfa getirdi
chat-act-delete = { $count } dosya sildi
chat-act-move = { $count } dosya taşıdı
chat-act-think = Düşündü
chat-act-other = { $count } başka adım
chat-steps-count = { $count } adım
chat-steps-last = Son: { $step }
chat-steps-open = Adımları göster
chat-files-title = { $count } dosya değişti
chat-files-more = Ve { $count } tane daha
chat-plan-label = Plan
chat-error-label = Bir şey ters gitti
prompt-hint = Enter gönderir, Shift+Enter yeni satır açar.
prompt-watching = Burada izleyebilirsiniz. Bu ajanla bir üye ya da sahibi konuşur.
session-started-with = { $name }: { $value }
session-started-hint = Bu ajan başlatılırken seçildi. Ajan bunu bildirmez, bu yüzden sonradan değişmiş olabilir.
empty-changes-title = Her şey temiz
empty-changes-hint = Bir ajanın değiştirip kaydetmediği dosyalar burada görünür.
empty-messages-title = Henüz mesaj yok
empty-messages-hint = Ajanlar birbirine yazdığında burada izleyebilirsiniz.
# Ekip sohbeti (sağ panel): "codex · teal  alıcı: claude · plum  2 dk. önce", metin iki satırda kesilir; satır tüm metni açar.
team-chat-to = alıcı: { $name }
team-chat-show-all = Tümünü göster
team-chat-close = Kapat
empty-board-title = Henüz ortak görev yok
empty-board-hint = Ajanların bir projede paylaştığı görevler ve dosyalar burada görünür.
empty-usage-title = Henüz kullanım yok
empty-usage-hint = Bir ajan çalıştıktan sonra token, bağlam, süre ve plan limitleri burada görünür.
loading-label = Yükleniyor
chat-code = Kod
live-summary = Canlı özet
live-open-changes = Değişiklikleri aç
usage-total-tokens = { $short } token kullanıldı
usage-total-time = Başlangıçtan beri { $time } ajan süresi
usage-since-start = Başlangıçtan beri geçen süre
chat-steps-needs-you = Sizi bekliyor
chat-steps-failed = Hatayla bitti
prompt-closed = Bu ajan sona erdi, istek alamaz.
chat-thought-for = { $time } düşündü
chat-plan-progress = { $done } / { $total } tamam
chat-review-changes = Değişiklikleri incele
chat-diff-show-all = Tüm { $count } satırı göster
chat-state-since = { $state } · { $time }

# Araç bölmeleri: Terminal, Değişiklikler, Dosyalar ve Dosya yöneticisi çalışma alanının bölmeleri olarak (ADR-0068).
layout-tool-of = { $what } · { $name }
layout-panel-manager = Dosya yöneticisi
palette-open-terminal = Terminali yanında aç
palette-open-changes = Değişiklikleri yanında aç
palette-open-files = Dosyaları yanında aç
palette-open-manager = Dosya yöneticisini yanında aç
palette-new-terminal = Yeni terminal başlat
pane-new-terminal = Yeni terminal
pane-open-beside = Yanında aç
pane-open-beside-named = { $name } değişikliklerini yanında aç
pane-open-changes = Değişiklikleri bir bölmede aç
pane-switch-folder = Başka bir klasörü göster
pane-choose-folder = { $name } için bir klasör seçin
pane-choose-terminal = Gösterilecek bir terminal seçin
pane-no-folders-title = Henüz klasör yok
pane-no-folders-hint = Makinelerinizden birinde bir ajan bir klasörde başlayınca klasör burada görünür.
pane-no-terminals-title = Henüz terminal yok
pane-no-terminals-hint = Bir makinede ajan ya da terminal başlatın, burada gösterilebilir.
pane-offline-title = { $name } burada kullanılamıyor
pane-offline-hint = Bu pencere makinelerinize bağlı değil.
pane-no-folder-title = Bu ajan için klasör yok
pane-no-folder-hint = Makine henüz onun için bir klasör bildirmedi. Birazdan yeniden deneyin.
start-terminal-missing = Bu makinede düz terminal yok (“{ $kind }” türü). Ajan klasörüne bir “{ $kind }” bildirimi ekleyin ya da bir ajan seçin.
fm-col-name = Ad
fm-col-kind = Tür
fm-col-size = Boyut
fm-up = Üst klasör
fm-actions = Dosya eylemleri
fm-copy-path = Yolu kopyala
fm-copy-path-hint = Yolu klasöre göre kopyala
fm-copied = Yol kopyalandı
fm-copy-failed = Kopyalanamadı
fm-download = İndir
fm-download-none = İndirmek için bir dosya seçin
fm-download-failed = Bu dosya indirilemedi
fm-terminal = Ajan terminali
fm-terminal-root = Ajanın terminalini yanında aç
fm-terminal-subfolder = Terminal yalnızca ajanın kendi klasöründe açılır. Bu pencere bir alt klasörde terminal başlatamaz.
fm-terminal-closed = Bu ajan sona erdi, terminali yok.
fm-terminal-chat = Bu ajan bir sohbet; terminali yok.
fm-terminal-none = Burada açılacak bir terminal yok.
fm-rename = Yeniden adlandır
fm-move = Taşı
fm-delete = Sil
fm-upload = Yükle
fm-write-why = Dosyaları değiştirmek makinede onayınızı gerektirir ve bu pencere henüz onay isteyemez.
fm-folders = Klasörler
fm-list = Bu klasördeki dosyalar
fm-empty = Bu klasör boş.
fm-count = { $count ->
    [one] 1 öğe
   *[other] { $count } öğe
}
fm-preview = Önizleme
fm-back = Listeye dön
fm-close-preview = Önizlemeyi kapat
fm-binary = Bu dosya metin değil, burada gösterilmez. İndirebilirsiniz.
fm-kind-folder = Klasör
fm-kind-code = Kod
fm-kind-text = Metin
fm-kind-image = Görsel
fm-kind-archive = Arşiv
fm-kind-config = Ayar
fm-kind-file = Dosya
fm-kind-private = Gizli dosya
tool-status-pending = Başlamayı bekliyor
tool-status-running = Çalışıyor
tool-status-done = Bitti
tool-status-failed = Başarısız
tool-status-stopped = Durduruldu
tool-output-cut = Çıktı kısaltıldı.
chat-thinking = Düşünüyor…
chat-steps-call-failed = { $count ->
    [one] Bir adım başarısız oldu
   *[other] { $count } adım başarısız oldu
}
permission-pending = İzin istendi
permission-approved = İzin verildi
permission-denied = İzin verilmedi
permission-approved-always = Bu oturum için izin verildi
permission-approved-always-kept = İzin verildi; ajan bunu hatırlıyor
permission-cancelled = Yanıtlanmadı
permission-show = İsteği göster
chat-queued-mark = Bekliyor
chat-not-sent-mark = Gönderilmedi
session-effort = Çaba
session-on = Açık
session-off = Kapalı
session-more = Diğer ayarlar
prompt-queued = Bu turun bitmesi bekleniyor.
prompt-queue-full = Çok fazla mesaj bekliyor. Ajanı bekleyin ya da turunu durdurun.
prompt-placeholder-commands = Ne yapmalı? Komutlar için / yazın.
queue-count = { $count ->
    [one] 1 sırada
   *[other] { $count } sırada
}
queue-note = Bu tur bitince gönderilir
queue-drop = Sırayı boşalt
queue-drop-title = Bekleyen mesajlar silinsin mi?
queue-drop-body = { $count ->
    [one] Bu işlem ajanın o anki turunu da durdurur ve bekleyen mesaj gönderilmez. Ajanın kendisi çalışmaya devam eder.
   *[other] Bu işlem ajanın o anki turunu da durdurur ve bekleyen { $count } mesaj gönderilmez. Ajanın kendisi çalışmaya devam eder.
}
queue-drop-yes = Sırayı boşalt ve turu durdur
queue-drop-no = Kalsınlar
slash-menu = Ajanın sunduğu komutlar
turn-stop = Turu durdur
turn-stop-hint = Yalnızca bu turu bitirir. Ajan çalışmaya devam eder ve ona başka bir mesaj gönderebilirsiniz.
turn-stop-failed = Tur durmadı.
chat-trimmed = { $count ->
    [one] 1 eski kayıt yüklenmedi.
   *[other] { $count } eski kayıt yüklenmedi.
}
refusal-unsupported = Bu ajanın makinesi bunu henüz desteklemiyor.
refusal-unconfirmed = Bu seçim ajanın sormadan işlem yapmasına izin verir ve bu makine henüz onayınızı isteyemiyor. Makineyi güncelleyin veya ajanı o şekilde başlatın.
refusal-forbidden = Bu makinenin ayarları ajanın sormadan işlem yapmasına izin vermiyor.
refusal-unknown-option = Bu ayar ya da değer onayınıza sunulamıyor: ajan artık sunmuyor ya da adı güvenle gösterilemiyor.
refusal-limit = Bu makinede çok fazla istek bekliyor. Birini yanıtlayın veya bekleyin, sonra yeniden deneyin.
refusal-skip-agent-limit = Bu ajanın onayınızı bekleyen çok fazla isteği var. Bekleyeni yanıtlayın veya biraz bekleyin, sonra yeniden deneyin.
refusal-agent-gone = O ajan artık çalışmıyor.
refusal-task-taken = Bu görevi bir ajan zaten aldı ya da görev bitti.
refusal-task-blocked = Bu görev, önce başka bir görevin bitmesini bekliyor.
refusal-task-gone = Bu görev artık panoda yok.
give-refused-other-project = O ajan bu görevin projesine ait değil.
refusal-board-unavailable = Bu projenin panosuna şu anda ulaşılamıyor. Biraz sonra yeniden deneyin.

# ADR-0091: plan limitleri. Sayılar aracın kendi bildirdiğidir; hiçbiri hesaplanmaz. Para yok.
quota-window-five-hour = Beş saat
quota-window-seven-day = Yedi gün
quota-window-minutes = { $minutes } dakika
quota-percent = %{ $percent }
quota-chip = Limit %{ $percent }
quota-chip-short = %{ $percent }
quota-chip-title = { $kind }, { $what }: plan limitinin %{ $percent } kadarı kullanıldı.
quota-resets = { $time } içinde sıfırlanır ({ $when })
quota-reset-unknown = Sıfırlanma zamanı bildirilmedi
quota-observed = Okunma: { $when }
quota-stale = Güncel değil
quota-stale-note = Pencerenin kendi süresinden eski, artık doğru olmayabilir
quota-reset = Sıfırlandı
quota-reset-note = Pencere sıfırlandı ({ $when }); henüz yeni okuma yok
quota-unknown-line = Bu araçtan limit bilgisi yok: { $kinds }
quota-span-days = { $days } gün { $hours } sa
quota-span-hours = { $hours } sa { $minutes } dk
quota-span-minutes = { $minutes } dk
quota-hint = { $kind }: { $what } limiti neredeyse doldu (%{ $percent }). Yine de başlatabilirsiniz.

# Başlık çubuğu simgeleri ve Ayarlar sayfası ("Bu cihaz" ve "Makinelerim" bölümleri).
settings-link = Ayarlar
machines-link = Makineler
header-links = Ayarlar ve makineler
settings-title = Ayarlar
settings-sections = Ayar bölümleri
settings-group-device = Bu cihaz
settings-group-machines = Makinelerim
settings-device = Ad ve parmak izi
settings-passkeys = Geçiş anahtarı
settings-machines = Makineler
settings-machines-manage = Makineleri yönet

# Çalışan bir ajanı kapatma (ajan menüsü, komut paleti, sohbet ajanının mesaj kutusunda /exit ve /quit)
close-agent = Ajanı kapat
agent-actions = { $name } için işlemler
palette-close-agent = Ajanı kapat: { $name }
palette-give-task = Bir ajana ver: { $title }
close-title = { $name } kapatılsın mı?
close-body = Bu ajanı sonlandırır. Konuşması, siz kaldırana kadar kapananlar listesinde kalır.
close-working = Şu an çalışıyor: yürüttüğü tur kaybolur.
close-keep = Çalışmaya devam etsin
close-failed-prefix = Kapatılamadı: { $reason }
close-failed = bir şeyler ters gitti. Yeniden deneyin.
close-failed-forbidden = bu makinede ajan kapatma yetkiniz yok
close-failed-policy = bu makine buna izin vermiyor
close-failed-grant = bu makine önce izninizi istiyor. Açılan pencerede izin verin, kapanır
close-failed-trust = bu makine henüz bu tarayıcıya güvenmiyor
close-failed-offline = o makine çevrimdışı
close-failed-timeout = o makine zamanında yanıt vermedi
close-failed-gone = makine bu ajanı artık tanımıyor
close-failed-unsupported = o makine bunu henüz yapamıyor. Güncelleyin
close-failed-hub = hub'a ulaşılamadı

# Ayarlar → Ajanlar: her makinenin başlatabildiği ajanlar, hangilerinin kurulu olduğu ve başlatırken ne seçilebildiği.
settings-agents = Ajanlar
settings-subtitle = Aksi belirtilmedikçe bu tarayıcıda tutulur.
device-desc = Makineler yalnızca güvendikleri bir cihazdan gelen yanıtları kabul eder.
device-fingerprint-hint = Bir makine sorduğunda bunu karşılaştırın.
device-trust-on = { $count ->
    [one] 1 makinede güveniliyor
   *[other] { $count } makinede güveniliyor
}
device-trust-none = Henüz güvenilmiyor
device-others-desc = Agora'nızda oturum açmış tarayıcılar.
device-remove-named = { $name } cihazını kaldır
device-remove-title = { $name } kaldırılsın mı?
device-remove-body = Makineleriniz artık ondan gelen yanıtları kabul etmeyecek. Bu işlem buradan geri alınamaz.
device-remove-keep = Kalsın
device-remove-confirm = Kaldır
when-day-time = { $day }, { $time }
attention-settings-desc = Bir ajan beklediğinde bu pencerenin dikkatinizi nasıl çektiği. Yalnızca bu tarayıcıda tutulur.
attention-mode-label = Bir ajan size ihtiyaç duyduğunda
attention-mode-short-modal = Kart
attention-mode-short-banner = Şerit
attention-mode-short-off = Ek bir şey yok
attention-notify-hint = Ad eklemediğiniz sürece yalnızca bir ajanın size ihtiyaç duyduğunu söyler.
password-desc = Diğer makinelerdeki tarayıcılardan, kodlarıyla birlikte istenir. Bu makinenin kendi tarayıcısından asla istenmez.
password-desc-set = Diğer makinelerdeki tarayıcılardan istenir. Değiştirmek diğer tüm pencerelerin oturumunu kapatır.
password-state-set = Ayarlı
password-state-unset = Ayarlı değil
settings-machines-desc = Agora'nıza bağlı her makine ve bu tarayıcıya güvenip güvenmediği.
settings-machines-online = { $online } / { $total } çevrimiçi
settings-shared = Ajanlarla paylaşılanlar
shared-desc = Ajanlarınızın başlarken aldığı MCP sunucuları; bu cihaza güvenen her makinede. Bu cihaz imzalar.
shared-add = Sunucu ekle
shared-count = { $count ->
    [one] { $count } sunucu
   *[other] { $count } sunucu
}
shared-loading = Paylaşılanlar yükleniyor…
shared-load-failed = Hub yanıt vermedi. Bağlantı geri gelince yeniden deneyin.
shared-empty = Henüz bir şey paylaşılmadı. Buraya eklediğiniz sunucu ajanlara bir sonraki başlangıçlarında verilir.
shared-empty-no-device = Henüz bir şey paylaşılmadı. Paylaşmak için cihaz anahtarı olan bir tarayıcı gerekir; Ayarlar'ı kendi tarayıcınızda açın.
shared-kind-skill = Beceri
shared-scope-all = Her proje
shared-no-refusals = Hiçbir makine reddetmedi
shared-refused-count = { $count ->
    [one] { $count } makinede reddedildi
   *[other] { $count } makinede reddedildi
}
shared-refused-on = { $machine }: { $reason }
shared-refused-secret-needs-launcher = gizli değer isteyen adresli bir sunucu ajanlara henüz verilmiyor
shared-refused-env-not-allowed = bir programın hangi kodu çalıştıracağını değiştiren bir değişken (PATH ya da LD_PRELOAD gibi) ayarlıyor; hiçbir paylaşılan sunucu bunu ayarlayamaz
shared-refused-secret-not-allowed = bu makine o gizli değerin paylaşılan sunuculara gitmesine izin vermiyor (politikadaki share.secrets)
shared-refused-secret = { $secret } gizli değeri bu makinede yok
shared-refused-policy-off = bu makine paylaşılan sunucuları almıyor
shared-refused-command-not-allowed = bu makine o programa izin vermiyor
shared-refused-url-not-allowed = bu makine adresteki sunuculara izin vermiyor
shared-refused-signer-untrusted = bu makine onu yayınlayan cihaza güvenmiyor
shared-refused-older = daha yeni bir sürüm önce ulaştı
shared-refused-invalid = bu makine onu okuyamadı
shared-refused-other = reddedildi
shared-change = Değiştir
shared-remove-named = { $name } kaldır
shared-remove-title = { $name } paylaşımı durdurulsun mu?
shared-remove-body = Bundan sonra başlayan ajanlar onu almaz. Çalışan ajanlar durana kadar onu korur.
shared-remove-keep = Kalsın
shared-remove-confirm = Paylaşımı durdur
shared-form = Bir MCP sunucusu paylaş
shared-name = Ad
shared-runs = Çalışma biçimi
shared-runs-program = Bir program
shared-runs-address = Bir adres
shared-command = Program
shared-args = Argümanlar
shared-args-hint = Her satıra bir tane
shared-env = Ortam
shared-url = Adres
shared-headers = Başlıklar
shared-scope = Kapsam
shared-url-warning-title = Ajanlar kodunuzu oraya gönderebilir
shared-url-warning = Bir ajan projelerinizde okuduklarını bu adrese gönderebilir. Yalnızca buna güvendiğiniz bir sunucuyu paylaşın.
shared-secret-title = Gizli değere her makine kendisi karar verir
shared-secret = Bir makine bu sunucuyu ajanlarına yalnızca kendi politikası o gizli değerin paylaşılan sunuculara gitmesine izin veriyorsa (share.secrets) ve gizli değer makinede varsa verir. Gizli değer, sunucu başlarken makinede doldurulur ve hiçbir dosyaya yazılmaz. Gizli değer isteyen adresli sunucular henüz verilmiyor.
shared-publish = İmzala ve paylaş
shared-publish-change = Değişikliği imzala ve paylaş
shared-cancel = Vazgeç
shared-check-name = Ad küçük harf, rakam ve tire kullanır; en fazla 64.
shared-check-command = Sunucuyu hangi programın çalıştırdığını yazın.
shared-check-env = Her ortam satırını AD=değer olarak yazın, her ad bir kez.
shared-check-args-secret = Gizli değer argüman olamaz: makinedeki herkes bir programın argümanlarını okuyabilir. Ortama koyun.
shared-check-url = Adres https:// ile başlar ve boşluk içermez.
shared-check-headers = Her başlık satırını Ad: değer olarak yazın, her ad bir kez.
shared-check-hidden = Ekranda görünmeyen ya da metni ters çeviren bir karakter içeriyor. Yapıştırmak yerine yeniden yazın.
shared-error-device = Hub bu cihazın imzasını kabul etmedi. Bu tarayıcının cihazlarınızdan biri olduğunu kontrol edin (yukarıda, Bu cihaz).
shared-error-older = Bu ada daha yeni bir değişiklik bu arada paylaşıldı. Sayfayı yenileyip yeniden deneyin.
shared-error-limit = En fazla 64 öge paylaşılabilir. Önce birini kaldırın.
shared-error-invalid = Hub bunu yazıldığı haliyle reddetti. Adı, programı ve değerleri kontrol edin.
shared-error-owner = Sunucuları ajanlarla yalnızca sahip paylaşır.
shared-error-other = Gerçekleşmedi. Biraz sonra yeniden deneyin.
agents-desc = Her makinenin başlatabildikleri. Seçimler bir ajanı başlatırken yapılır; neler aldığını görmek için bir kartı açın.
agents-machine = Makine
agents-runs-short-terminal = Terminal
agents-runs-short-chat = Sohbet
agents-runs-short-adapter = Sohbet
agents-kind = Kimlik
agents-runs = Çalışma biçimi
agents-offers-skip = "Sorma" seçeneği sunar: izin sorularını atlayacak şekilde başlatılabilir. Asla sizin yerinize seçilmez.
agents-loading = Bu makineye soruluyor…
agents-none-known = Bu makine çevrimdışı ve hangi ajanlara sahip olduğunu bu tarayıcıya bildirmedi.
agents-empty = Bu makine hiç ajan listelemiyor.
agents-stale = Çevrimdışı · { $when } itibarıyla
agents-not-installed = Kurulu değil
agents-runs-terminal = Terminalde çalışır
agents-runs-chat = Sohbet
agents-runs-adapter = Adaptör üzerinden sohbet
agents-program-missing = Bu makine PATH içinde { $program } programını bulamıyor.
agents-setting-any = ajanın kabul ettiği herhangi bir değer
agents-setting-default = varsayılan { $value }
agents-options = Seçenekler
agents-option-skips = izin sorularını atlar
agents-group-here = Bu makinede
agents-group-add = Ajan ekle
agents-group-add-desc = Bu makinede kurulu değil. Programı oraya kurun, sonra başlatılabilir; burada hiçbir şey çalışmaz ya da indirilmez.
agents-untested = Denenmedi
agents-untested-desc = ACP kayıt defterinden. Henüz kimse Agora'da denemedi: başlamayabilir ya da bazı özellikleri eksik olabilir.
agents-runs-fetch = Başlatmak { $program } çalıştırır; o da bu sürümü ilk seferde indirir.
agents-version = Sürüm
agents-license = Lisans
agents-repository = Depo
agents-website = Web sitesi
agents-source = Listeleyen
agents-source-registry = ACP kayıt defteri

# Bileşen galerisindeki bileşen takımı (visual-language.md §3). Yalnızca galeri için örnek metin.
kit-title = Bileşen takımı
kit-buttons = Düğmeler
kit-states = Durumlar ve etiketler
kit-avatars = Ajan simgeleri
kit-cards = Kartlar, satırlar ve ayarlar
kit-controls = Denetimler
kit-counts = Sayılar
kit-notices = Bildirimler ve istekler
kit-empty = Boş durumlar
kit-copy = Kopyalanacak komut
kit-rtl = Sağdan sola
kit-sample-forget = Unut
kit-sample-shared = Paylaşılan
kit-sample-notify = Arka planda bildirimler
kit-sample-notify-hint = Adlar yalnızca siz isterseniz görünür.
kit-sample-ask = { $name } bir komut çalıştırmak istiyor
kit-sample-offline = Bu makine çevrimdışı
kit-sample-offline-line = Ajanları çalışmayı sürdürür ve makine dönünce durumlarını bildirir.
kit-sample-joined = Makine katıldı. Makinelerim listesinde görünüyor.

# Yeniden tasarımın giriş ekranları (ADR-0102): giriş kartı, boş Ana sayfa ve komut paleti.
signin-or = ya da
copy-selected = Kopyalanamadı. Metin seçildi: klavyenizle kopyalayın.
fleet-add-machine = Makine ekle
palette-empty-hint = Başka bir sözcük deneyin: bir ajan, bir makine ya da bir eylem.
palette-group-go = Şuraya git
palette-group-actions = Eylemler
palette-group-agents = Ajanlar
palette-group-places = Makineler ve projeler
palette-group-tasks = Görevler
palette-group-appearance = Görünüm
palette-key-move = Gezin
palette-key-open = Aç
palette-key-close = Kapat

# Eklentiler (ADR-0114 yapım adımı 2, docs/ux/plugins.md).
palette-plugins = Eklentiler
plugin-panel = Eklenti
plugin-marker = Eklenti
plugin-development = geliştirme
plugin-frame-title = { $name } (eklenti)
plugins-title = Eklentiler
plugins-intro = Hub'ın sahibinin kurduğu eklentiler ve bir dosyadan yüklediğiniz eklentiler (yalnızca bu tarayıcıda) her biri kendi korumalı bölmesinde çalışır. Yalnızca izin verdiklerinizi görürler; anahtarlarınızı, ağı ve sizi bekleyenleri asla göremezler. Birini bir bölmede göstermek için bölmenin Göster… menüsünü kullanın.
plugins-safe = Güvenli kip: hiçbir eklenti çalışmıyor ve yerleşik tema açık. Eklentilerinizi yeniden kullanmak için güvenli kipten çıkın.
plugins-done = Tamam
plugin-load = Dosyadan yükle…
plugin-version-by = { $version }, { $author }
plugin-licence = Lisans: { $licence }
plugin-held = Bu sürüm, izin verdiğinizden fazlasını görmek istiyor. Siz yeniden bakana kadar bölmeleri kapalı kalır.
plugin-sees = { $count ->
    [0] Sizden hiçbir şey görmüyor.
    [one] İzin verdiğiniz bir şeyi görüyor.
   *[other] İzin verdiğiniz { $count } şeyi görüyor.
}
plugin-review = Yeniden bak
plugin-change = Neleri görebilir
plugin-remove = Kaldır
plugin-added = { $name } eklendi. Bir bölmede göstermek için bölmenin Göster… menüsünü kullanın.
plugin-added-visit = { $name } yalnızca bu ziyaret için eklendi: bu tarayıcı onu saklayamadı.
plugin-grant-title = { $name } eklensin mi?
plugin-grant-never = Yalnızca kendi bölmesinin içine çizer. Ağa, anahtarlarınıza, terminallerinize ve sizi bekleyenlere erişemez; sizin yerinize hiçbir şey yapamaz.
plugin-grant-asks = Görmek istedikleri:
plugin-grant-nothing = Sizden hiçbir şey görmek istemiyor.
plugin-cancel = Vazgeç
plugin-add = Eklentiyi ekle
plugin-save = Kaydet
plugin-from-hub = bu hub'da kurulu
plugin-not-shown = Gösterilmiyor. Neleri görmek istediğine bakın, isterseniz gösterin.
plugin-look = Bak
plugin-hide = Göstermeyi bırak
plugin-grant-title-hub = { $name } gösterilsin mi?
plugin-show = Göster
plugin-grant-failed = Hub seçiminizi saklamadı. Pencereyi yeniden açıp bir kez daha deneyin.
plugin-replaces-hub = Bu tarayıcıda hub'ın { $name } eklentisinin yerini, burada izin verdiklerinizle alır. Hub'ınkini yeniden kullanmak için bunu kaldırın.
plugin-replaced = Bu tarayıcıda aynı kimlikli bir geliştirme eklentisi bunun yerini aldı. Bunu yeniden kullanmak için o eklentiyi kaldırın.
plugin-catalog-title = Hub'ın kataloğundan
plugin-catalog-intro = Hub'ın sahibinin bu hub'daki herkes için kurabileceği, incelenmiş topluluk eklentileri. Hub her birini kataloğun imzasına karşı denetledi.
plugin-catalog-none = Hub'ın kataloğundaki her şey zaten kurulu.
plugin-catalog-asks = Görmek istedikleri: { $what }
plugin-catalog-unknown = { $count ->
    [one] bu pencerenin bilmediği bir şey
   *[other] bu pencerenin bilmediği { $count } şey
}
plugin-catalog-install-how = Onu yalnızca hub'ın sahibi, hub'ın makinesinde kurar:
plugin-catalog-upgrade-how = Kurulu olandan daha yeni bir sürüm. Onu yalnızca hub'ın sahibi, hub'ın makinesinde yükseltir:
plugin-cap-agents-read = Ajanlarınızın adları, türleri ve durumları, makineleri ve projeleriyle birlikte
plugin-cap-usage-read = Ajanlarınızın kullandığı token, bağlam ve süre
plugin-cap-tasks-read = Projelerinizin ortak görevleri
plugin-cap-changes-read = Bölmesinin gösterdiği ajanın klasöründe değişen dosyalar ve içlerinde neyin değiştiği
plugin-cap-chat-read = Bölmesinin gösterdiği ajanın konuşması, sırlar gizlenmiş olarak
plugin-cap-theme-read = Temanızın renkleri, onlara uyabilmesi için
plugin-cap-pane-context = Bölmesinin hangi ajanı gösterdiği ya da bölmenin neyi izlediği
plugin-error-too-large = Bu dosya 2 MB'tan büyük; bir eklenti en fazla bu kadar olabilir.
plugin-error-format = Bu dosya bir eklenti paketi değil.
plugin-error-newer-format = Bu eklenti paketi daha yeni bir sürüm için hazırlanmış.
plugin-error-too-many = Bu paket, bir eklentinin içerebileceğinden fazla dosya içeriyor.
plugin-error-path = Bu paket, adında klasör ya da yol olan bir dosya içeriyor ({ $detail }); bu yüzden hiçbir parçası yüklenmedi.
plugin-error-link = Bu paket bir bağlantı ya da dosya olmayan başka bir şey içeriyor ({ $detail }); bu yüzden hiçbir parçası yüklenmedi.
plugin-error-type = Bu paket, bir eklentinin kullanamayacağı türde bir dosya içeriyor ({ $detail }); bu yüzden hiçbir parçası yüklenmedi.
plugin-error-manifest = Paketin tanımı (agora-plugin.json) eksik ya da "{ $detail }" alanında hatalı.
plugin-error-capability = Bu eklenti, bu pencerenin bilmediği bir şeyi görmek istiyor ({ $detail }); bu yüzden yüklenmedi.
plugin-error-unlisted = Bu paket, tanımında listelenmeyen bir dosya içeriyor ({ $detail }); bu yüzden hiçbir parçası yüklenmedi.
plugin-error-missing = Paketin tanımında listelenen bir dosya eksik ({ $detail }); bu yüzden hiçbir parçası yüklenmedi.
plugin-error-hash = Bu paketteki bir dosya, tanımında listelenen dosya değil ({ $detail }); bu yüzden hiçbir parçası yüklenmedi.
plugin-error-newer = Bu eklenti { $detail } ya da daha yeni bir sürüm gerektiriyor.
plugin-error-full = Zaten dört eklenti yüklü. Önce birini kaldırın.
plugin-file-kind = Agora eklenti paketi
plugin-reload-on = Dosyası değişince yeniden başlar.
plugin-reload-unsupported = Bu tarayıcı bir dosyayı izleyemiyor: değişen bir eklentiyi elle yeniden yükleyin (Chrome ve Edge kaydedince yeniden başlatır).
plugin-reload-refused = { $name } değişti, ancak yeni paket yüklenmedi: { $reason } Denetimden geçen son sürüm çalışmaya devam ediyor.
plugin-reload-other-id = Dosyada artık başka bir eklenti var ({ $id }).
plugin-reload-unreadable = Dosya artık okunamıyor; yeniden yükleyin.
safe-mode-on = Güvenli kip: eklenti yok, yerleşik tema açık.
safe-mode-leave = Güvenli kipten çık
plugin-frame-use = { $name } (eklenti): kullanmak için tıklayın ya da Enter'a basın
safe-mode-open = Güvenli kipte aç
palette-safe-mode = Güvenli kipte aç (eklenti yok, yerleşik tema)
plugin-stopped = { $name } durduruldu: pencereye çok fazla ileti gönderdi.
plugin-restart = Yeniden başlat

# Bir ajan bitirdiğinde ya da sana ihtiyaç duyduğunda çalan sesler (Ayarlar > Bu cihaz > Sesler; ajanın menüsü) ve görülene dek kalan bitti işareti
sounds-title = Sesler
sounds-desc = Bir ajan bitirdiğinde ya da size ihtiyaç duyduğunda kısa bir ses. Yalnızca bu tarayıcıda tutulur.
sounds-label = Ses çal
sounds-hint = Siz açana dek kapalı. Pencere aynı şeyi her zaman ekranda da gösterir.
sounds-on-hint = Sesler bu pencere arka plandayken çalar, baktığınız ajan için asla çalmaz.
sounds-unsupported = Bu tarayıcı ses çalamıyor.
sounds-safe-mode = Güvenli kipte ses yok.
sounds-volume = Ses düzeyi
sounds-volume-low = Kısık
sounds-volume-medium = Orta
sounds-volume-high = Yüksek
sounds-finished = Bir ajan bitirdiğinde
sounds-finished-hint = Bir ajan sırasını bitirdiğinde iki yumuşak, yükselen nota.
sounds-needs-you = Bir ajan size ihtiyaç duyduğunda
sounds-needs-you-hint = Bir ajan sizi beklediğinde ya da onayınızı istediğinde iki kısa nota.
sounds-try = Çal
sounds-try-named = Sesi çal: { $name }
sounds-in-front = Bu pencere öndeyken de
sounds-in-front-hint = Kullandığınız bölmedeki ajan için yine asla çalmaz.
app-notify-title = Uygulama bildirimleri
app-notify-desc = Bir ajan bitirdiğinde ya da size ihtiyaç duyduğunda bu uygulamadan bir bildirim. Yalnızca bu uygulamada tutulur.
app-notify-label = Bildirim göster
app-notify-hint = Siz açana dek kapalı. Yalnızca bu uygulama arka plandayken.
app-notify-on-hint = Yalnızca bu uygulama arka plandayken. Ajanın adını ve ne olduğunu söyler, başka bir şey değil; birine tıklamak ajanı açar. Sisteminiz bu uygulamanın bildirim gösterip gösteremeyeceğini bir kez sorabilir.
app-notify-safe-mode = Güvenli kipte bildirim yok.
app-notify-finished = Bir ajan bitirdiğinde
app-notify-needs-you = Bir ajan size ihtiyaç duyduğunda
attention-notify-app = Uygulamada aşağıdaki Uygulama bildirimlerini kullanın.
agent-sounds-mute = Bu ajanın seslerini kapat
finished-unseen = Siz bakmıyorken bitti. Açınca bu işaret kalkar.
finished-unseen-short = Yeni
palette-sounds-on = Sesleri aç
palette-sounds-off = Sesleri kapat
keys-cmd-sounds = Sesleri aç ya da kapat
keys-cmd-mute-agent = Odaktaki ajanın seslerini kapat ya da aç
palette-agent-mute = { $name } için sesleri kapat
palette-agent-unmute = { $name } için sesleri aç

## Galeri (ADR-0120): iki topluluk kataloğu tek ekranda.
catalog-gallery-title = Galeri
catalog-gallery-open = Galeriyi aç
catalog-gallery-back = Geri
catalog-gallery-show = Göster
catalog-gallery-themes = Temalar { $count }
catalog-gallery-plugins = Eklentiler { $count }
catalog-gallery-search-themes = Temalarda ara
catalog-gallery-search-plugins = Eklentilerde ara
catalog-gallery-sort = Sırala
catalog-gallery-sort-popular = Popüler
catalog-gallery-sort-new = Yeni
catalog-gallery-sort-name = Ad
catalog-gallery-sort-author = Yazar
catalog-gallery-intro = Agora sahibinin açtığı kataloglardan. Burada gösterilen her şey çizilmeden önce bu pencere tarafından denetlenir.
catalog-gallery-popularity = Popülerlik { $count }
catalog-gallery-popularity-hint = Popülerlik, kataloğun yayınladığı haliyle. Agora kendisi hiçbir şey saymaz.
catalog-gallery-added = Eklenme { $day }
catalog-gallery-unavailable = Katalog şu an gösterilemiyor: pencere bir hub'a bağlı değil ya da hub galeriden eski.
catalog-gallery-no-match = Aramanızla eşleşen bir şey yok.
catalog-gallery-preview-loading = Önizleme yükleniyor…
catalog-gallery-preview-refused = Önizleme yok: bu pencere temayı reddetti.
catalog-gallery-use = Kullan
catalog-gallery-details = Ayrıntılar
catalog-gallery-close = Kapat
catalog-gallery-scheme = Görünüm
catalog-gallery-dark = Koyu
catalog-gallery-light = Açık
catalog-gallery-you-have = Sizde { $version }. sürüm var; yalnızca siz güncellediğinizde değişir.
catalog-gallery-theme-install-note = Kurmak, temayı kendi temalarınıza kopyalar; oturum açtığınız her cihazda olur. Bir temadaki hiçbir şey kod çalıştıramaz.
catalog-gallery-plugins-off = Bu hub'da topluluk eklenti kataloğu kapalı. Sahibi agora hub plugins on ile açabilir.
catalog-gallery-plugin-not-added = Eklenmedi
catalog-gallery-plugin-newer = { $version } çıktı (bu Agora'da { $from } var)
catalog-gallery-plugin-added = Bu Agora'ya eklendi
catalog-gallery-plugin-added-note = Bu sürüm bu Agora'ya eklendi. Neleri görebileceğini seçmek için Eklentiler'i açın.
catalog-gallery-asks = Görmek istedikleri
catalog-gallery-copy-command = Komutu kopyala
catalog-gallery-copied = Kopyalandı
palette-gallery = Galeri: temalar ve eklentiler

board-col-review = İnceleme
board-col-review-empty = İnceleme bekleyen görev yok.
board-review-read = Raporu oku
board-review-return = Bekleyenlere geri gönder
board-review-accept = Kabul et
board-review-read-failed = Rapor yüklenemedi. Tekrar deneyin.
board-review-decision-failed = Karar kaydedilemedi. Görevi kontrol edip tekrar deneyin.
