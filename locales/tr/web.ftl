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
palette-placeholder = Bir ajana git veya bir işlem yap…
palette-empty = Eşleşen yok.
palette-open-agent = { $name } aç
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
device-title = Bu cihaz
device-intro = Makineler yalnızca sahibinin güvendiği cihazların yanıtını kabul eder. Buna güvenmek için makinede şunu çalıştırın:
device-name = Cihaz adı
device-key = Cihaz anahtarı
device-link = Bu cihaz

signin-title = Giriş yap
signin-intro = Hub'ınızdan bir kod isteyin: hub'ı çalıştıran makinede "agora hub login" komutunu çalıştırıp kodu buraya yazın.
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
start-limit = Makine kapasitesine ulaştı. Bir ajanı durdurup yeniden deneyin.
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
board-title = Ortak görevler ve dosyalar
board-open = açık
board-done = bitti
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
prompt-failed = Gitmedi. Yeniden deneyin.
console-history = Geçmiş
history-title = { $name }: commit'ler
history-none = Burada henüz commit yapılmadı.
history-failed = Geçmiş o makineden okunamadı.
history-no-subject = (mesaj yok)
chat-empty = Henüz bir şey söylenmedi. Aşağıdan ona ne yapacağını söyleyin.
chat-log = Konuşma
chat-tool = Araç
chat-plan = Plan
