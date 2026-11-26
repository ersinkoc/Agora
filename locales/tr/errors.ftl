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
node-policy_denied = { $rule ->
        [registry_signed_start] { $kind } başlarken programını indirir; bu yüzden onu yalnızca güvendiğiniz cihazlardan birinde imzalanan bir başlatma başlatabilir (penceredeki Başlat ya da terminal istemcisinde klasörde başlatma), hub tek başına başlatamaz.
        [registry_off] Bu makinenin politikası ACP kayıt defterinin ajanlarını kapatıyor (`agents.registry`: `off`); { $kind } başlatılmadı.
        [registry_denied] Bu makinenin politikası { $kind } ajanını kapatıyor (`agents.registry.deny`).
       *[other] { $node_id } makinesi buna izin vermiyor (kural: { $rule }).
    }
node-resource_limit = { $resource } için makine sınırına ulaşıldı (sınır: { $limit }).
project-not_found = { $project_id } projesi bulunamadı.
worktree-dirty = { $path } içinde kaydedilmemiş değişiklikler var.
worktree-exists = { $branch } dalı için bir çalışma alanı zaten var.
worktree-unmerged = { $branch } dalındaki değişiklikler başka hiçbir yerde kayıtlı değil.
agent-not_found = { $agent_id } ajanı bulunamadı.
agent-kind_unknown = { $kind } ajan türü kullanılamıyor.
agent-publisher_revoked = { $kind } bağdaştırıcısı, kayıt defterinin iptal ettiği { $publisher } yayımcısından geliyor. Kaldırın ya da başka bir yayımcıdan kurun.
agent-sandbox_unavailable = Bu makine ajanları kısıtlayacak şekilde ayarlı ({ $level }), ancak { $kind } burada kısıtlanamıyor; bu yüzden başlatılmadı.{ $reason ->
        [broad_write_grant] {" "}Kullanacağı bir klasöre her kullanıcı yazabiliyor.
        [control_token_reachable] {" "}Verilecek bir klasör, bu makinedeki Agora'nın sizi ajanlardan ayırmak için kullandığı anahtarı içeriyor; ajan sizin yerinize işlem yapabilirdi. Bu makinenin Agora klasörünü ajanların kullanabildiği klasörlerin dışında tutun; agora doctor hangisi olduğunu söyler.
        [label_failed] {" "}Kullanacağı bir klasör kısıtlı ajanlar için işaretlenemiyor.
        [protected_path_unresolved] {" "}Bu makinenin kendi Agora klasörü bulunamıyor; ajana kapalı tutulamazdı.
        [protected_path_reachable] {" "}Verilecek bir klasör bu makinenin kendi Agora dosyalarını (verilerini ya da ajan türlerini) içeriyor ya da onlara götürüyor ve bu sistem onları ajanın kullanabildiği bir klasörün içinde kapalı tutamıyor. Onları oradan taşıyın (/tmp, /var ya da /opt altına koymayın); neden bir ayarsa agora doctor hangisi olduğunu söyler. Hiçbirini söylemiyorsa, ajana verilen bir klasör başlarken onlara giden bir bağlantıydı: o bağlantıyı denetleyip kaldırın.
        [secrets_reachable] {" "}Verilecek bir klasör bu makinenin sakladığı gizli bilgileri içeriyor. O klasörü bu makinenin ajan kurallarından çıkarın.
        [writable_is_link] {" "}Kullanabileceği bir dosya ya da klasöre, yazabileceği bir yerdeki bağlantı üzerinden ulaşılıyor; o bağlantıyı kendisi yapmış olabilir. Bağlantıyı kontrol edin: siz yapmadıysanız silin. Agora böyle bağlantıları, sizin yaptıklarınızı da (bir dotfiles bağlantısı) izlemez.
        [writable_has_links] {" "}Yazabileceği bir dosyanın başka bir yerde de bir adı var (sabit bağlantı); ona yazmak orayı da değiştirirdi. O bağlantıyı silin (ya da yerine bir kopya koyun) ve yeniden başlatın. node_modules, target ve .git içindeki bağlantılar denetlenmez.
        [worktree_not_folder] {" "}Klasörü artık düz bir klasör değil (yerine bağlantı gibi bir şey konmuş).
       *[other] {""}
    }
agent-not_authenticated = { $kind }, { $node_id } makinesinde giriş yapmamış.
agent-blocked = { $agent_id } ajanı girdi bekliyor, bu yüzden mesaj gönderilmedi.
agent-mode_unsupported = { $kind }, { $mode } modunu desteklemiyor.
agent-option_forbidden = { $kind }, { $option } seçeneğiyle başlatılmadı: { $policy ->
        [confined_only] bu makine bir ajanın sormadan çalışmasına yalnızca klasörüyle sınırlıyken izin veriyor ve burada sınırlanamazdı.
        [signature] böyle başlatmak için başlatma isteğinin cihazınızca imzalanması gerekir.
       *[other] bu makine ajanların sormadan çalışmasına izin vermiyor.
    }
agent-option_invalid = { $kind } başlatılmadı: "{ $option }" başlatma seçeneği onun sunduğu bir seçenek değil (ya da çok fazla seçenek var).
agent-setting_unknown = { $kind } başlatılmadı: bu makinede "{ $setting }" adlı bir ayarı yok.
agent-setting_invalid = { $kind } başlatılmadı: "{ $setting }" için seçilen değer kabul ettiği değerlerden biri değil.
agent-option_unconfirmed = { $kind } için { $option } seçeneği ajanın sormadan çalışmasını sağlar ve bu istek bunun için onayınızı taşımıyordu.
agent-running = { $agent_id } hâlâ çalışıyor. Önce durdurun.
node-online = { $node_id } bağlı. Yalnızca çevrimdışı bir makine unutulabilir.
agent-not_interrupted = { $agent_id } yarıda kalmadı ya da artık devam ettirilmek üzere sunulmuyor.
agent-session_missing = { $agent_id } kendi oturumuna devam edemiyor. Bunun yerine yeniden başlatın.
agent-folder_changed = { $agent_id } ajanının klasörü yok ya da artık başka bir projeye ait: { $path }
agent-still_running = { $agent_id } makinenin düğümü yeniden başlamadan önceki haliyle hâlâ çalışıyor. Önce durdurun, sonra devam ettirin.
agent-parent_not_running = { $agent_id } ajanını { $parent_agent_id } başlattı. Önce o ajanı devam ettirin.
approval-expired = { $approval_id } onayının süresi doldu.
approval-signature_invalid = { $approval_id } onayı doğrulanamadı.
approval-grant_required = Bu makine önce sizden izin istiyor: { $command }.
approval-scope_forbidden = Bu makine ajanların sormadan iş yapmasına izin vermiyor; yalnızca bu istek için izin verilebilir.
approval-scope_unavailable = Ajan bu istek için bu yanıtı sunmuyor. Hiçbir şey gönderilmedi; yeniden yanıtla.
approval-always_only_forbidden = Ajan “{ $action }” için izin istedi ve yalnızca hatırlayacağı bir yanıt sundu. Bu makine yalnızca tek seferlik yanıtlara izin veriyor, bu yüzden reddedildi.
approval-adapter_outdated = Bu ajanı Agora’nın eski bir sürümü başlattı; tek seferlik bir izni hatırlayabilir. Bu makine yalnızca tek seferlik yanıtlara izin veriyor: ajanı yeniden başlat, sonra yanıtla.
device-unknown = Hub'da sizin { $device_id } adlı bir cihazınız yok.
device-key_mismatch = { $device_id } cihazı başka bir anahtarla biliniyor.
device-untrusted = { $device_id } cihazına { $node_id } makinesinde güvenilmiyor.
device-signature_invalid = { $device_id } cihazı için imza doğrulanamadı.
device-endorsement_expired = { $device_id } cihazının onayının süresi dolmuş ya da çok uzun.
device-not_in_scope = { $device_id } cihazının onayı { $node_id } makinesi için değil.
device-revoked = { $device_id } cihazı onaylandıktan sonra kaldırıldı.
lease-held = { $path }, { $expires_at } zamanına kadar { $holder_agent_id } ajanı tarafından kullanılıyor.
board-unavailable = { $project_id } için ortak panoya şu anda ulaşılamıyor. Çalışmanız bu makinede devam ediyor.
board-not_allowed = Bu hub'ı başka kişiler de kullandığı için ajanlara her makineyi ve ajanı göstermiyor. Hub'ın sahibi bunu açabilir.
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
mesh-unavailable = Ağ yeniden bağlanıyor. Biraz sonra tekrar deneyin.
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
files-secret = { $path } gizli bir dosya ve gösterilmez.
adapter-signature_invalid = { $package } paketi doğrulanamadı.
adapter-crashed = { $kind } beklenmedik şekilde durdu (çıkış kodu { $exit_code }).
adapter-agent_exited = { $stage ->
        [turn] Ajan çalışırken kapandı (çıkış kodu { $exit_code }). Yeniden başlatın. Son çıktısı: { $stderr_tail }
       *[other] Ajan hazır olmadan kapandı (adım { $stage }, çıkış kodu { $exit_code }). Yeniden başlatın. Son çıktısı: { $stderr_tail }
    }
adapter-agent_refused = Ajan başlamayı reddetti (adım { $stage }).
adapter-option_refused = Ajan { $id } seçeneğine geçmedi.
adapter-permission_unanswerable = Ajan “{ $action }” için izin istedi ama izin vermenin bir yolunu sunmadı, bu yüzden reddedildi.
adapter-no_answer = { $call ->
        [approval] { $kind } yanıtınızı { $waited_ms } ms içinde almadı. İstek hâlâ açık: biraz bekleyip yeniden yanıtlayın.
        [session] { $kind }, harekete geçmeden önce sorup sormadığını { $waited_ms } ms içinde bildirmedi ve bu makine yalnızca soran ajanlara izin veriyor; bu yüzden ona hiçbir şey gönderilmedi.
       *[prompt] { $kind } isteğinizi { $waited_ms } ms içinde onaylamadı. Biraz bekleyin; bir şey olmazsa yeniden gönderin.
    }
pair-closed = Bu hub şu anda yeni makine kabul etmiyor.
pair-limit = Çok fazla eşleştirme denemesi. { $seconds } saniye sonra yeniden deneyin.
pair-expired = Eşleştirme çok uzun sürdü ve iptal edildi.
pair-mismatch = Eşleştirme doğrulanamadı ve iptal edildi.
pair-denied = Sahibi isteği reddetti.
pair-unknown = Bu eşleştirme yok ya da sona erdi.
pair-state = Eşleştirme o adımda olmadığı için iptal edildi ({ $state }).
pair-insecure_transport = Bu şekilde giriş yapmak için hub'a güvenli bir bağlantı (https) gerekir.
pair-name_unverified = { $name } adresindeki hub doğrulanamadı, bu yüzden hiçbir şey kaydedilmedi.
pair-names_missing = Hub, sertifikasının yeni bir makinenin kullanacağı adresi içerdiğinden emin olamadı ({ $reason }: { $names }). Hiçbir şey açılmadı. Hub'ı, yeni bir makinenin kullanacağı her adres için `--name ADRES` ile başlatın, örneğin `--name 192.168.1.20`.
pair-discovery_unconfirmed = Ağda bulunan bir hub yalnızca bir terminalde onayladığınızda kullanılır. Hiçbir şey kaydedilmedi. `agora join` komutunu bir terminal penceresinde çalıştırın ya da hub'ın adresini verin: `agora join 192.168.1.20:7443`.
pair-ca_mismatch = Hub, onayladığınız sertifika otoritesini sunmadı (duyurduğu parmak izi, teslim ettiğininkiyle aynı değil), bu yüzden bu makine ona katılmadı. Hiçbir şey kaydedilmedi.
preview-port_refused = { $port } numaralı bağlantı noktası önizlenemez ({ $reason }). Yalnızca bu makinenin kendi adresinde, 1024 ve üzerinde, Agora'nın kendisine ait olmayan bir bağlantı noktasındaki program gösterilebilir.
preview-limit = Bu makine izin verilen en çok önizlemeye ulaştı ({ $limit }). Önce birini kapatın.
preview-expired = { $forward_id } önizlemesi sona erdi. Devam etmek için yeniden açın.
preview-unknown = Bu makinede { $forward_id } adlı bir önizleme yok.
preview-unavailable = Önizleme bu kurulumda çalışmıyor ({ $reason }). Hub üzerinde kendine ait bir adres gerekir.
agent-option_unknown = Ajanın bu değerle { $id } adlı bir ayarı yok.
agent-queue_full = Bu ajan için zaten { $limit } mesaj bekliyor. Ajanın bitirmesini bekleyin ya da geçerli turunu durdurun.
agent-approval_needed = Bu seçim ajanın sormadan çalışmasını sağlar, bu yüzden onayınızı bekliyor ({ $approval_id }). Henüz hiçbir şey değişmedi.
enroll-renew_refused = Bu makinenin sertifikası yenilenemedi ({ $reason }). Elindeki geçerli kalır; iptal edildiyse ya da süresi dolduysa hub'a yeniden katılın.
task-dispatch_limited = Son bir dakikada ajanlara { $limit } kez görev vermeyi denediniz. Biraz bekleyip yeniden deneyin.
agent-program_missing = { $kind } başlamadı: bu makine { $program } programını PATH içinde bulamıyor. Programı oraya kurun ya da düğümü { $program } komutunun çalıştığı bir terminalden başlatın.
theme-catalog_off = Bu hub'da topluluk tema kataloğu kapalı. Sahibi agora hub themes on ile açabilir.
theme-not_found = Tema kataloğunda { $id } adlı bir tema yok. Son güncellemede kaldırılmış olabilir.
plugin-not_found = { $plugin_id } eklentisi bu hub'da kurulu değil ya da bu sürümüyle kurulu değil.
plugin-catalog_off = Bu hub'da topluluk eklenti kataloğu kapalı. Sahibi agora hub plugins on ile açabilir.
settings-too_large = Temalarınız Agora'nızda saklanamayacak kadar büyük (en fazla { $max } bayt). Bu tarayıcıda kalıyorlar.
rate-limited = Bir dakikada çok fazla değişiklik (en fazla { $limit }). Hiçbir şey değişmedi; biraz bekleyip yeniden deneyin.
settings-stale = Temalarınız az önce başka bir cihazda değişti (sürüm { $version }). Birleştirilip yeniden kaydedilecek.
internal = Bir şeyler ters gitti. Olay numarası: { $incident_id }.
