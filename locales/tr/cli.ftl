# Komut satırı çıktısı. Makine tarafından okunan veriler (kimlikler, durumlar, yollar) olduğu gibi yazılır; yalnızca cümleler burada.

cli-usage =
    Kullanım:
      agora node run [--socket YOL] [--manifests DİZİN] [--project YOL]... [--data DİZİN] [--worktrees DİZİN]
      agora agent start TÜR [--project AD] [--name AD] [--socket YOL]
      agora agent stop AJAN_KİMLİĞİ [--socket YOL]
      agora agent list [--socket YOL]
      agora terminal attach AJAN_KİMLİĞİ [--socket YOL]
      agora node stdio [--socket YOL]
      agora mesh stdio --socket YOL
      agora hub run [--listen ADDR] [--hub-dir DIR] [--name HOST]...
      agora hub code [--hub-dir DIR]
      agora node enroll --hub URL --ca FILE --code CODE --id NAME
      agora doctor [--data DİZİN] [--manifests DİZİN] [--socket YOL]
cli-node-listening = Düğüm çalışıyor. Denetim soketi: { $socket }
cli-agent-started = { $agent_id } ajanı { $worktree } içinde başlatıldı
cli-agent-stopped = { $agent_id } ajanı durduruldu
cli-agent-list-empty = Bu düğümde ajan yok.
cli-terminal-detached = { $agent_id } ajanından ayrıldınız.
cli-attach-needs-terminal = Bağlanmak için etkileşimli bir terminal gerekir.
cli-node-not-running = { $socket } adresinde dinleyen bir düğüm yok. "agora node run" ile başlatın.
cli-unknown-project = Bu düğümde { $project } adında kayıtlı proje yok.
cli-ambiguous-project = Birden fazla proje kayıtlı; --project ile birini seçin.
cli-no-projects = Bu düğümde proje yok. --project YOL ile başlatın.
cli-bad-arguments = Argümanlar anlaşılamadı.
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
doctor-manifest-invalid = { $detail } ajan tanımı okunamadı.
doctor-summary-ok = Agora'nın ihtiyaç duyduğu her şey yerinde.
doctor-summary-problems = Agora'nın ihtiyaç duyduğu bir şey eksik. SORUN olarak işaretlenenleri düzeltin.

cli-hub-listening = Hub { $addr } adresinde çalışıyor. Veri klasörü: { $dir }
cli-hub-failed = Hub komutu başarısız oldu: { $reason }
cli-hub-code = Kayıt kodu: { $code } (10 dakika geçerli). Diğer makinede şunu çalıştırın: agora node enroll --hub wss://ADRES:PORT --ca { $ca } --code { $code } --id AD
cli-enrolled = Bu makine { $id } adıyla kaydedildi. "agora node run" ile başlatın; hub'a kendiliğinden bağlanır. Kayıt yeri: { $dir }.
cli-enroll-refused = Hub bu makineyi kaydetmeyi reddetti ({ $code }). Kod yanlış, süresi dolmuş veya zaten kullanılmış olabilir.
