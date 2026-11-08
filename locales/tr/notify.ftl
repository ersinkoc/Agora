# Bildirim metinleri, alıcının dilinde üretilir. Sade sözcükler, iç terim yok.

notify-agent-blocked = { $agent } sizi bekliyor.
notify-agent-done = { $agent } bitti.
notify-approval-requested = { $agent } onayınızı istiyor.
notify-task-completed = Bir görev tamamlandı: { $task }.
notify-node-offline = { $machine } çevrimdışı oldu.
notify-budget-threshold = Harcama bütçenizin %{ $percent } düzeyine ulaştı.
notify-update-available = Bir güncelleme var: { $version }.
notify-digest =
    { $count ->
        [one] Siz yokken 1 gelişme oldu.
       *[other] Siz yokken { $count } gelişme oldu.
    }

notify-title = Agora
notify-test = Agora bildirimleri çalışıyor.
