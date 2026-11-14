# Terminal istemcisi (ADR-0043). Arayüz sözlüğü: makine, "Sizi bekliyor", "Bitti" (docs/ux/visual-language.md).
tui-connecting = { $hub } adresine bağlanılıyor…
tui-lost = { $hub } ile bağlantı koptu. Yeniden deneniyor…
tui-signin-failed-local = Bu makinenin hub'ında oturum açılamadı. `agora` komutunu yeniden çalıştırın; `agora status` çalışıp çalışmadığını ve günlüğünün yerini gösterir.
tui-signin-failed = Hub kodu kabul etmedi. "agora hub login" ile yeni bir kod alın.
tui-need-hub = Hub'ınızın adresini belirtin: bu komutu hub'ın çalıştığı makinede çalıştırın ya da --hub ADRES:PORT --ca DOSYA --code KOD verin.
tui-quit-hint = Çıkmak için Ctrl-C
tui-header = Agora · { $total } makineden { $online } tanesi çevrimiçi · { $agents } ajan çalışıyor
tui-attention = Sizi bekliyor
tui-nothing-waiting = Sizi bekleyen bir şey yok.
tui-asks = { $agent } soruyor: { $action }
tui-asks-start = { $node } üzerindeki { $agent } ({ $agent_kind }), { $folder } klasöründe{ $worktree ->
        [true] , kendi çalışma ağacında
       *[other] {""}
    } { $kind } başlatmak istiyor (derinlik { $depth }). İlk istemi:
tui-asks-start-outside = { $node } üzerindeki { $agent } ({ $agent_kind }), kendi klasörünün dışında, { $folder } klasöründe{ $worktree ->
        [true] , kendi çalışma ağacında
       *[other] {""}
    } { $kind } başlatmak istiyor (derinlik { $depth }). İlk istemi:
tui-started-by = { $agent } tarafından başlatıldı
tui-blocked = { $agent } sizi bekliyor
tui-machines = Makineler
tui-online = çevrimiçi
tui-offline = çevrimdışı
tui-last-seen = son görülme { $unit ->
        [s] { $n } sn
        [min] { $n } dk
        [h] { $n } sa
       *[d] { $n } gün
    } önce
tui-no-agents = Henüz ajan yok. Web uygulamasından ya da "agora agent start" ile bir tane başlatın.
tui-state-working = Çalışıyor
tui-state-needs-you = Sizi bekliyor
tui-state-idle = Boşta
tui-state-finished = Bitti
tui-state-unknown = Bilinmiyor
tui-no-project = (proje yok)
tui-hint-answer = Yukarıdaki ilk isteğe izin vermek için y, reddetmek için n tuşuna basın.
tui-allowed = İzin verildi.
tui-denied = Reddedildi.
tui-not-trusted = Bu cihaza o makinede henüz güvenilmiyor. O makinede şunu çalıştırın: agora node trust { $id } { $key }   (`agora` orada bulunamazsa komutu agora programının klasöründe çalıştırın: ./agora, Windows'ta .\agora.exe). Ya da başka bir cihazınızdan bunu onaylamasını istemek için a tuşuna basın (parmak izi { $fingerprint }).
tui-device-name = { $host } üzerindeki terminal
tui-device-asked = İstendi. Bu terminali ({ $fingerprint }) makinelerinizin güvendiği başka bir cihazınızda onaylayın: pencerede ya da onun terminal istemcisinde v ile.
tui-devices-none-waiting = Onay bekleyen bir cihazınız yok. Bu terminalin parmak izi: { $fingerprint }
tui-device-cannot-vouch = Henüz hiçbir makine bu terminale güvenmiyor; onayı hiçbir yerde geçmez.
tui-device-card =
    { $name } makinelerinizde güvenilir olmak istiyor.
    Parmak izi: { $fingerprint } (o cihazın gösterdiğiyle karşılaştırın)
    Bu terminale güvenilen yerlerde ona da güvenilecek: { $machines }
    Bu terminalin anahtarıyla onaylamak için y'ye, vazgeçmek için başka bir tuşa basın.
tui-device-approved = { $name } onaylandı. Her makine onayı kendisi denetler; denetleyince cihaz güvenilir görünür.
tui-answer-failed = Yanıtınız gönderilemedi ({ $code }).
tui-hint-open = Tab: sonraki ajan · Enter: aç · x: durdur · s: yeni başlat (Ctrl-] geri döner)
tui-opening-failed = Bu terminal açılamadı ({ $code }).
tui-usage = { $input } giriş tokenı · { $output } çıkış tokenı
tui-chat-you = Siz
tui-chat-agent = { $agent }
tui-chat-tool = çalıştırdı: { $text }
tui-chat-plan = Plan
tui-chat-diff = değiştirdi: { $path }
tui-chat-input = ›
tui-chat-hint = Bir istek yazıp Enter'a basın · /mode ya da /model ve bir ad geçiş yapar · Ctrl-] geri döner
tui-chat-mode = mod { $mode }
tui-chat-model = model { $model }
tui-chat-context = bağlam %{ $percent }
tui-notice-turn_failed = (sıra başarısız oldu, hata { $rpc_code }; sağlayıcısı isteği reddetmiş olabilir) { $stderr_tail }
tui-notice-turn_stopped = (ajan sırasını bitirdi: { $stop_reason })
tui-notice-turn_empty = (sıra tek kelime söylenmeden bitti; sağlayıcısı isteği reddetmiş olabilir) { $stderr_tail }
tui-notice-agent_exited = (ajan durdu, adım { $stage }, çıkış kodu { $exit_code }: { $stderr_tail })
tui-notice-agent_refused = (ajan başlamayı reddetti, adım { $stage }, hata { $rpc_code }: { $detail })
tui-notice-other = (ajan bunu bildirdi: { $code })
tui-option-switched = Geçildi.
tui-option-unknown = Ajan { $id } seçeneğini sunmuyor. Sunduğu: { $ids }
tui-option-confirm = { $id } modunda ajan size sormadan çalışır. Yine de geçmek için aynı satırı tekrar yazın.
tui-stopped = Durduruldu.
tui-stop-failed = Durdurulamadı ({ $code }).
tui-stop-helpers = Başlattığı { $count } yardımcı hâlâ çalışıyor. Onlar da dursun mu? Çalışmaya devam etmeleri için n'ye, durmaları için başka bir tuşa basın.
tui-send-failed = Gönderilemedi ({ $code }).
tui-start-pick = { $machine } makinesinde hangi ajan başlatılsın? Numarasına basın; başka bir tuşa basarsanız işlem iptal edilir.
tui-start-item = { $n }  { $name }
tui-start-none = { $machine } makinesinde kurulu bir ajan yok.
tui-start-missing = Bu makinede bulunamadı: { $names }. Makine bu programları PATH içinde bulamıyor. Onları oraya kurun ya da düğümü bu programların çalıştığı bir terminalden başlatın.
tui-start-nowhere = Projesi olan çevrimiçi bir makine yok.
tui-started = { $agent } başlatıldı.
tui-start-failed = Başlatılamadı ({ $code }).
tui-cost = yaklaşık { $usd } USD
tui-grant-ask = O makine bu işlemi yapmanıza izin vermeden önce onayınızı istiyor. Bir saatliğine izin vermek için y tuşuna basın; başka bir tuşa basarsanız işlem iptal edilir.
tui-granted = Bir saatliğine izin verildi.
tui-split-answer = y izin verir, n ilk isteği reddeder
tui-chat-answer = y izin verir, n reddeder · cevaplanana kadar yazamazsın
tui-note-copy = c komutu kopyalar · herhangi bir tuş bunu kapatır
tui-note-close = Herhangi bir tuş bunu kapatır.
tui-copied = Komut panoya kopyalandı.
tui-split-asks-start = { $kind } başlatmak istiyor
tui-split-card-more = … { $count } satır daha: tamamını okumak için Enter
tui-split-read-first = İzin vermeden önce isteğin tamamını okuyun: Enter'a basın.
tui-reader-keys = j / k ya da oklar kaydırır · boşluk bir sayfa iner · q geri döner
tui-split-pick = Soldan bir ajan seç
tui-split-no-screen = Terminali açılıyor…
tui-split-status = { $online }/{ $total } makine · { $agents } ajan · { $waiting } seni bekliyor
tui-split-keys = ↑↓ seç · Enter aç · s başlat · f klasör · x durdur · u devam et · d kaldır · a onay iste · v cihazlar · q çık
tui-split-chat = Terminali yok: Enter konuşmasını açar
tui-folder-title = { $machine } üzerinde bir klasörde ajan başlat
tui-folder-keys = ↑↓ seç · Enter aç · ⌫ yukarı · Boşluk burada başlat · m makine · Esc vazgeç
tui-folder-git = git
tui-folder-empty = Burada klasör yok.
tui-folder-more = … gösterilenden fazla klasör var (ilk { $n })
tui-folder-unsupported = Bu makine klasörlerini henüz listeleyemiyor (güncelleyin).
tui-folder-failed = O klasör açılamadı ({ $code }).
tui-folder-identity = Bu klasörün projesi son ajandan beri değişti. Yine de başlatmak için y, vazgeçmek için başka bir tuş.
tui-forgot = Listeden kaldırıldı.
tui-forget-running = Hâlâ çalışıyor: önce durdur (x).
tui-forget-failed = Kaldırılamadı: { $reason }
tui-forget-disconnected = Bağlantı kesildi. Yeniden bağlanmasını bekleyin.
tui-forget-timeout = Zamanında yanıt alınamadı. Tekrar deneyin.
tui-forget-action = ajanı kaldırma
tui-interrupted = makinesiyle birlikte durdu
tui-interrupted-resume = makinesiyle birlikte durdu · u devam et · U yeniden başlat
tui-interrupted-again = makinesiyle birlikte durdu · U yeniden başlat
tui-resumed = { $agent } yeniden çalışıyor.
tui-resume-not-interrupted = Yalnızca makinesiyle birlikte duran bir ajan devam ettirilebilir (u) ya da yeniden başlatılabilir (U).
tui-resume-failed = Başlamadı: { $reason }
tui-resume-cannot = Kendi konuşmasını sürdüremiyor. U onu klasöründe yeniden başlatır (yeni bir konuşma).
tui-resume-confirm = { $agent } ajanını devam ettirmek için 5 saniye içinde yeniden u'ya basın.
tui-again-confirm = { $agent } ajanını klasöründe yeniden başlatmak için 5 saniye içinde yeniden U'ya basın (yeni bir konuşma).
tui-interrupted-running = makinesi yeniden başlamadan önceki haliyle hâlâ çalışıyor · x durdurur
tui-resume-still-running = Yeniden başlamadan önceki işlemi hâlâ çalışıyor. Önce x ile durdurun, sonra devam ettirin.

# ADR-0059: launch options (manifest `[[launch.option]]` labels)
option-skip-permissions = Sormadan
option-ask = Önce sorsun
tui-start-acts = Nasıl çalışsın? Numarasına basın; Enter ya da başka bir tuş önce sordurur.
tui-start-confirm-skip = Ajan sormadan çalışsın mı? "{ $name }" seçiliyken çalıştığı sürece komut çalıştırır ve dosyaları size sormadan değiştirir. Sormadan başlatmak için y'ye basın; başka bir tuş sormaya devam ettirir.
tui-badge-no-ask = sormuyor

## The app shell (ADR-0063 stage 2)
tui-app-keys-tree = Ctrl K komutlar · Enter aç · N seni bekleyen sonraki · y / n cevapla · s başlat · f klasör · x durdur · u devam · d kaldır · a onay iste · v cihazlar · Tab sonraki bölüm · i ayrıntılar · q çık
tui-app-keys-terminal = Ctrl-] ya da F12 listeye dön · Tuşların ajana gider · Alt+1…9 sekmeler
tui-app-keys-inspector = Tab sonraki bölüm · Esc listeye dön · Ctrl K komutlar
tui-app-keys-palette = Süzmek için yaz · ↑↓ seç · Enter çalıştır · Esc kapat
tui-app-mouse-on = fare açık (metin seçmek için m ile bırak)
tui-app-mouse-off = fare kapalı (m ile geri al)
tui-app-no-agent = Soldan bir ajan seçip Enter'a basın; terminali burada görünür.
tui-app-no-screen = Bu ajanın terminali açılıyor.
tui-app-tree-empty = Henüz ajan yok.
tui-app-palette-title = Komutlar
tui-app-palette-none = Eşleşen yok.
tui-app-cmd-open = { $name } aç
tui-app-cmd-next = Seni bekleyen sonraki ajana git
tui-app-cmd-allow = Bekleyen isteğe izin ver (y)
tui-app-cmd-deny = Bekleyen isteği reddet (n)
tui-app-cmd-start = Ajan başlat (s)
tui-app-cmd-folder = Bir klasörde ajan başlat (f)
tui-app-cmd-stop = Seçili ajanı durdur (x)
tui-app-cmd-resume = Seçili ajana devam et (u)
tui-app-cmd-again = Seçili ajanı yeniden başlat (U)
tui-app-cmd-remove = Seçili ajanı kaldır (d)
tui-app-cmd-ask = Bu terminalin onayını diğer cihazlarından iste (a)
tui-app-cmd-devices = Bekleyen bir cihazı onayla (v)
tui-app-cmd-mouse = Fareyi aç ya da kapat
tui-app-cmd-inspector = Ayrıntıları göster ya da gizle
tui-app-cmd-close = Bu sekmeyi kapat
tui-app-cmd-quit = Çık
tui-app-start-read = İsteğin tamamını okudunuz. İzin vermek için y, reddetmek için n.
