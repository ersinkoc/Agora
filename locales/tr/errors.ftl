# Hata iletileri. Anahtar = hata kodunun noktaları tire yapılmış hâli (docs/specs/error-codes.md).

auth-unauthenticated = Giriş yapmadınız.
auth-forbidden = Bunu yapmaya izniniz yok ({ $action }).
auth-device_revoked = Bu cihaz ({ $device_id }) kaldırıldı ve artık bağlanamaz.
protocol-unsupported_generation = Bu sürüm uyumlu değil: sunulan { $offered }, desteklenen { $supported }.
protocol-capability_missing = Karşı taraf { $capability } özelliğini desteklemiyor.
protocol-invalid_message = İstek geçerli değildi (alan: { $field }).
protocol-producer_mismatch = { $node_id } makinesi başka bir adla ({ $producer_id }) kaydedilmiş olaylar gönderdi; hub bunları kabul etmedi.
node-offline = { $node_id } makinesi bağlı değil.
node-unknown = { $node_id } makinesi bu hub tarafından tanınmıyor.
node-timeout = { $node_id } makinesi zamanında yanıt vermedi.
node-policy_denied = { $node_id } makinesi buna izin vermiyor (kural: { $rule }).
node-resource_limit = { $resource } için makine sınırına ulaşıldı (sınır: { $limit }).
project-not_found = { $project_id } projesi bulunamadı.
worktree-dirty = { $path } içinde kaydedilmemiş değişiklikler var.
worktree-exists = { $branch } dalı için bir çalışma alanı zaten var.
worktree-unmerged = { $branch } dalındaki değişiklikler başka hiçbir yerde kayıtlı değil.
agent-not_found = { $agent_id } ajanı bulunamadı.
agent-kind_unknown = { $kind } ajan türü kullanılamıyor.
agent-publisher_revoked = { $kind } bağdaştırıcısı, kayıt defterinin iptal ettiği { $publisher } yayımcısından geliyor. Kaldırın ya da başka bir yayımcıdan kurun.
agent-sandbox_unavailable = Bu makine ajanları kısıtlayacak şekilde ayarlı ({ $level }), ancak { $kind } burada kısıtlanamıyor; bu yüzden başlatılmadı. { $reason ->
        [broad_write_grant] Kullanacağı bir klasöre her kullanıcı yazabiliyor.
        [label_failed] Kullanacağı bir klasör kısıtlı ajanlar için işaretlenemiyor.
       *[other] {""}
    }
agent-not_authenticated = { $kind }, { $node_id } makinesinde giriş yapmamış.
agent-blocked = { $agent_id } ajanı girdi bekliyor, bu yüzden mesaj gönderilmedi.
agent-mode_unsupported = { $kind }, { $mode } modunu desteklemiyor.
agent-running = { $agent_id } hâlâ çalışıyor. Önce durdurun.
approval-expired = { $approval_id } onayının süresi doldu.
approval-signature_invalid = { $approval_id } onayı doğrulanamadı.
approval-grant_required = Bu makine önce sizden izin istiyor: { $command }.
lease-held = { $path }, { $expires_at } zamanına kadar { $holder_agent_id } ajanı tarafından kullanılıyor.
board-unavailable = { $project_id } için ortak panoya şu anda ulaşılamıyor. Çalışmanız bu makinede devam ediyor.
mesh-recipient_unknown = { $to } adında bir ajan veya kanal yok.
mesh-rate_limited = Çok hızlı mesaj gönderiyorsunuz. Biraz sonra yeniden deneyin.
mesh-body_too_large = Mesaj çok büyük (sınır: { $limit }). Bunun yerine bir referans gönderin.
mesh-too_many_refs = Çok fazla referans var (sınır: { $limit }).
mesh-too_many_asks = Yanıt bekleyen çok fazla soru var (sınır: { $limit }).
mesh-timeout = Zamanında yanıt gelmedi.
mesh-folder_not_allowed = Yardımcı yalnızca isteyen ajanın kendi klasöründe çalışabilir: { $folder } bunun dışında.
mesh-helper_limit = Ajan şu anda başka bir yardımcı başlatamaz (sınır: { $limit }). { $rule ->
        [depth] Bir yardımcı kendi yardımcılarını başlatamaz.
        [pending] Başka bir başlatma hâlâ yanıtınızı bekliyor.
        [prompt] İlk istemi çok uzun.
       *[other] Zaten izin verilen sayıda yardımcı çalıştırıyor.
    }
mesh-start_denied = Başlatma reddedildi ({ $approval_id }).
task-not_found = { $task_id } görevi bulunamadı.
task-not_open = { $task_id } görevi zaten alınmış veya bitmiş.
task-blocked = { $task_id } görevi { $depends_on } görevini bekliyor.
task-not_owner = { $task_id } görevi başka bir ajana ait.
lease-not_found = { $lease_id } kilidi bulunamadı.
lease-invalid_path = { $path } kilitlenemez.
files-not_found = { $path } bulunamadı.
files-too_large = { $path } çok büyük (sınır: { $limit }).
files-outside_root = { $path } proje klasörünün dışında.
adapter-signature_invalid = { $package } paketi doğrulanamadı.
adapter-crashed = { $kind } beklenmedik şekilde durdu (çıkış kodu { $exit_code }).
adapter-agent_exited = Ajan hazır olmadan kapandı (adım { $stage }, çıkış kodu { $exit_code }). Son çıktısı: { $stderr_tail }
adapter-agent_refused = Ajan başlamayı reddetti (adım { $stage }).
adapter-option_refused = Ajan { $id } seçeneğine geçmedi.
internal = Bir şeyler ters gitti. Olay numarası: { $incident_id }.
