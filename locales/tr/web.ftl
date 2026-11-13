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

agent-on-machine = { $machine } üzerinde
agent-workspace = Çalışma alanı: { $name }
machines-online =
    { $count ->
        [one] { $count } makine çevrimiçi
       *[other] { $count } makine çevrimiçi
    }
link-connecting = Hub'a bağlanılıyor…
link-offline = Hub'a ulaşılamıyor. Yeniden deneniyor…
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
start-approval-title = { $node } üzerindeki { $name } ({ $agentKind }), { $kind } başlatmak istiyor
start-approval-depth = Derinlik
start-approval-folder = Klasör
start-approval-worktree = kendi çalışma ağacında
start-approval-outside = Bu klasör, isteyen ajanın kendi klasörünün dışında.
start-approval-prompt = İlk istemi
start-approval-no-prompt = İstem yok: başlar ve bekler.
start-approval-scope = Neye izin verilsin
start-approval-once = Yalnızca bu
start-approval-no-grant = Yalnızca bu başlatmaya izin verilebilir: bu tür kendisine verilen her programı çalıştırır, bu yüzden hiçbir zaman sormadan başlatılmaz.
start-approval-more = { $name } çalıştığı sürece bu klasörde, yeniden sormadan en fazla { $count } { $kind } ajanı
start-approval-nested = Yardımcıları da yardımcı başlatmak isteyebilir (her birini siz yanıtlarsınız)
helper-started-by = { $name } tarafından başlatıldı
helper-grants = Yardımcı izinleri
helper-grant-line = Sormadan hâlâ { $count } { $kind } başlatabilir, klasör:
helper-grant-revoke = Geri al
helper-grants-failed = Yardımcı izinleri okunamadı ya da değiştirilemedi.
device-title = Bu cihaz
device-intro = Makineler yalnızca sahibinin güvendiği cihazların yanıtını kabul eder. Bu cihazı güvenilir olarak kaydetmek için makinede şunu çalıştırın:
device-name = Cihaz adı
device-key = Cihaz anahtarı
device-link = Bu cihaz

signin-title = Giriş yap
signin-intro = Hub'ınızdan bir kod isteyin: hub'ı çalıştıran makinede "agora hub login" komutunu çalıştırıp kodu buraya yazın.
signin-code = Kod
signin-action = Giriş yap
signin-refused = Bu kod kabul edilmedi. Yanlış, süresi dolmuş veya daha önce kullanılmış olabilir.
signin-too-many = Çok fazla yanlış kod girildi. Bir dakika bekleyip yeniden deneyin.
signin-failed = Hub'ınıza ulaşılamadı. Yeniden deneyin.
machines-title = Makineleriniz
machines-empty = Henüz bağlı makine yok.
machine-online = Çevrimiçi
machine-offline = Çevrimdışı
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
agent-cost = yaklaşık { $amount }
machine-cost = yaklaşık { $amount } harcandı
usage-total = Toplam yaklaşık { $amount } harcandı
usage-prices-date = fiyatlar { $date } tarihli
usage-prices-bundled = fiyatlar bu hub sürümüyle gelen tablodan
board-title = Ortak görevler ve dosyalar
board-open = açık
board-done = bitti
board-project-identity = Proje kimliği
board-project-folders = Makineler ve klasörler ({ $count })
messages-title = Son mesajlar
message-cross = makineler arası
usage-projects = Projeye göre harcama
console-agents = Ajanlar
console-activity = Etkinlik
console-wall-empty = Terminalini burada izlemek için soldan bir ajan seçin.
console-maximize = Tam boyut
console-hide = İzlemeyi bırak
console-no-messages = Ajanlar henüz birbirine yazmadı.
console-spent = Şimdiye kadarki harcama (tahmini)
changes-title = Git'e kaydedilmemiş değişiklikler
changes-none = Git'e kaydedilmemiş değişiklik yok. Tüm çalışma alanları temiz.
changes-files = { $count ->
    [one] 1 dosya değişti
   *[other] { $count } dosya değişti
}
diff-title = { $name } neleri değiştirdi
diff-close = Kapat
diff-failed = Değişiklikler o makineden okunamadı.
console-files = Dosyalar
files-title = { $name } dosyaları
files-tree = Klasörler ve dosyalar
files-pick = Okumak için soldan bir dosya seçin.
files-binary = Bu dosya metin olmadığı için gösterilmiyor.
files-too-large = Bu dosya okunamadı (çok büyük olabilir).
files-failed = Bu klasör listelenemedi.
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
chat-tool = Araç
chat-plan = Plan
notice-adapter-turn_failed = Ajanın sırası başarısız oldu (hata { $rpc_code }). Sağlayıcısı isteği reddetmiş olabilir; yeniden deneyin ya da başka bir model seçin. { $stderr_tail }
notice-adapter-turn_stopped = Ajan sırasını kendisi bitirdi ({ $stop_reason }).
notice-adapter-turn_empty = Ajan hiçbir şey söylemeden sırasını bitirdi. Sağlayıcısı isteği reddetmiş olabilir. { $stderr_tail }
notice-adapter-agent_exited = Ajan durdu (adım { $stage }, çıkış kodu { $exit_code }). Son çıktısı: { $stderr_tail }
notice-adapter-agent_refused = Ajan başlamayı reddetti (adım { $stage }, hata { $rpc_code }): { $detail }
notice-other = Ajan bu pencerenin gösteremediği bir şey bildirdi ({ $code }).
session-mode = Mod
session-model = Model
session-not-reported = Bildirilmedi
session-mode-unasked = { $name } (sormadan)
session-context = Bağlam %{ $percent }
session-context-under-one = Bağlam %1'in altında
session-context-detail = Ajanın bağlamında { $size } jetondan { $used } jeton
session-cost = Ajanın bildirdiği: { $amount }
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
ide-tab-cost = Harcama
theme-label = Renk düzeni
theme-system = Sistemi izle
ide-shared = Paylaşılan
ide-memory = Bu makinedeki ajanların kullandığı bellek
network-hub = Hub'ınız
network-build = Bu makinede çalışan sürüm
network-never = hiç
network-heard = { $when } duyuldu
network-last-seen = son duyulma: { $when }
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
closed-section = Kapananlar
closed-clear = Kapananları temizle
closed-dismiss = Bu listeden kaldır
closed-forget = Herkes için kaldır
closed-ok = Tamamlandı
closed-failed = Çıkış koduyla bitti: { $code }
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

# ADR-0059: launch options (manifest `[[launch.option]]` labels)
option-skip-permissions = Sormadan
