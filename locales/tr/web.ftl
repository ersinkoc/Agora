# Web istemcisi metinleri. Arayüz sade sözcükler kullanır, iç terimler kullanmaz.

app-name = Agora
home-title = Ana sayfa
gallery-title = Bileşen galerisi
attention-title = Sizi bekleyenler
attention-empty = Her şey sakin. Sizi bekleyen yok.
fleet-title = Ajanlarınız
fleet-empty = Henüz ajan yok. Başlatın, burada görünsün.

state-working = Çalışıyor
state-needs-you = Sizi bekliyor
state-finished = Bitti
state-idle = Boşta
state-unknown = Bilinmiyor

agent-on-machine = { $machine } üzerinde
agent-workspace = Çalışma alanı { $name }
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
palette-placeholder = Bir ajana git veya bir eylem çalıştır…
palette-empty = Eşleşen yok.
palette-open-agent = { $name } aç
palette-next-attention = Sıradaki bekleyen
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
device-title = Bu cihaz
device-intro = Makineler yalnızca sahibinin güvendiği cihazların yanıtını kabul eder. Buna güvenmek için makinede şunu çalıştırın:
device-name = Cihaz adı
device-key = Cihaz anahtarı
device-link = Bu cihaz

signin-title = Giriş yap
signin-intro = Hub'ınızdan bir kod isteyin: onu çalıştıran makinede "agora hub login" komutunu çalıştırıp kodu buraya yazın.
signin-code = Kod
signin-action = Giriş yap
signin-refused = Bu kod çalışmadı. Yanlış, süresi dolmuş veya daha önce kullanılmış olabilir.
signin-too-many = Çok fazla yanlış kod. Bir dakika bekleyip yeniden deneyin.
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
start-limit = Makine sınırında. Bir ajanı durdurup yeniden deneyin.
start-failed = Ajan başlamadı. Yeniden deneyin.
signin-passkey = Geçiş anahtarıyla giriş yap
signin-passkey-failed = Geçiş anahtarı çalışmadı. Bunun yerine kod kullanın.
device-passkey-intro = Bu cihazda kodsuz giriş yapmak için bir geçiş anahtarı ekleyin.
device-passkey-add = Geçiş anahtarı ekle
device-passkey-added = Geçiş anahtarı eklendi. Bir dahaki sefere "Geçiş anahtarıyla giriş yap" seçeneğini kullanın.
device-passkey-failed = Geçiş anahtarı eklenemedi.
agent-cost = yaklaşık { $amount } (tahmini)
machine-cost = yaklaşık { $amount } harcandı (tahmini)
usage-total = Toplam yaklaşık { $amount } harcandı (tahmini)
usage-prices-date = fiyatlar { $date } tarihli
usage-prices-bundled = fiyatlar bu hub sürümüyle gelen tablodan
messages-title = Son mesajlar
message-cross = makineler arası
