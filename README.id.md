<p align="center"><img src="assets/hero.png" alt="itda: sebuah tangan menyerahkan kartu situs web kecil kepada agen AI lewat seutas benang. Cukup ketik alamat setelah itda dan AI Anda bisa memakai situsnya." width="100%"></p>

<p align="center"><strong>Cukup ketik alamat setelah itda dan AI Anda bisa memakai situsnya.</strong></p>

<p align="center"><a href="README.md">English</a> | <a href="README.ko.md">한국어</a> | <a href="README.ja.md">日本語</a> | <a href="README.zh-Hans.md">简体中文</a> | <a href="README.zh-Hant.md">繁體中文</a> | <a href="README.es.md">Español</a> | <a href="README.fr.md">Français</a> | <a href="README.de.md">Deutsch</a> | <a href="README.pt-BR.md">Português (Brasil)</a> | <a href="README.vi.md">Tiếng Việt</a> | <b>Bahasa Indonesia</b> | <a href="README.ru.md">Русский</a></p>

<p align="center">
  <a href="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml"><img src="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/brnyxx/itda-cli/releases"><img src="https://img.shields.io/github/v/release/brnyxx/itda-cli?include_prereleases&amp;label=release" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License: Apache-2.0"></a>
  <a href="https://brnyxx.github.io/itda-cli/docs/">Dokumentasi</a>
</p>

Beri itda satu alamat. itda mempelajari apa yang bisa dilakukan situs itu dan
mencobanya. itda memberi agen AI Anda daftar singkat aksi dalam bahasa
sehari-hari. Pembacaan langsung jalan. Tidak ada yang berubah di situs sampai
Anda bilang ya. itda berjalan di komputer Anda sendiri, dan juga berfungsi pada
intranet lama di balik halaman masuk.

itda ditujukan untuk situs yang dijalankan perusahaan Anda dan layanan tempat
Anda punya akun, bukan web secara keseluruhan.

<p align="center"><img src="assets/setup.gif" alt="Jalankan pertama: itda meminta alamat, news.ycombinator.com ditempel, itda menemukan daftar berisi 30 rekaman, satu formulir dan satu feed, mencoba kedua pembacaan, menyimpan situs dan menghubungkan Claude Desktop setelah satu Enter" width="860"></p>

<details>
<summary>Jalannya yang sama, sebagai teks</summary>

```console
$ itda news.ycombinator.com
1/5  Melihat news.ycombinator.com...
    Halaman ini terbuka tanpa masuk.
    Ditemukan: daftar berisi 30 rekaman, satu formulir, satu feed.
2/5  Yang akan bisa dilakukan agen:
    baca home         List what Hacker News shows  (masukan: p)
    baca feed         Read the RSS feed
3/5  Mencoba setiap pembacaan sekali...
    ✓ home         30 rekaman
    ✓ feed         ok
4/5  Disimpan sebagai 'news-ycombinator'.
    Hubungkan Claude Desktop? [Y/n]
5/5  Menambahkan itda ke Claude Desktop...
itda ditambahkan ke Claude Desktop (~/Library/Application Support/Claude/claude_desktop_config.json).
Buka Claude Desktop dan itda akan ada di sana.

Selesai. Minta agen Anda melakukan sesuatu di 'news-ycombinator', misalnya "pakai news-ycombinator untuk List what Hacker News shows".
```

</details>

Minta agen Anda: *"pakai news-ycombinator untuk menampilkan daftar Hacker
News"*. Situs di balik formulir masuk akan meminta nama pengguna dan kata sandi
Anda. itda mengisi formulir milik situs itu sendiri, sebagai dirinya sendiri,
dan menyimpan sesinya di komputer Anda. Jika itu tidak cukup (single sign-on,
kode sekali pakai), browser Anda sendiri terbuka. Anda masuk dan memakai situs
seperti biasa sementara itda mengamati.

<p align="center"><img src="assets/flow.svg" alt="Rekam situs web sekali di browser Anda; itda menyusun draf perintah darinya. Agen memanggilnya; sebuah perubahan menunggu persetujuan orang." width="860"></p>

> Status: masih awal, dan dipakai pada situs nyata. Runtime, CLI, server MCP,
> persetujuan (di terminal atau di dalam aplikasi agen), log audit, penyusunan
> draf dari dokumen OpenAPI, rekaman browser atau halaman yang diambil, kunci
> API, masuk lewat browser dan langkah pendahuluan CSRF sudah berfungsi hari
> ini. Paket situs bertanda tangan menyusul. Lihat
> [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Mengapa

Agen yang mengendalikan browser membaca ulang setiap halaman dan menghabiskan
ribuan token pada setiap tangkapan layar atau cuplikan aksesibilitas. Mereka
mengklik apa pun yang disuruh halaman dan rusak diam-diam saat tata letak
berubah. itda memindahkan pekerjaan itu ke langkah pemasangan:

| | Agen pengendali browser | itda |
|---|---|---|
| Yang dilihat agen | Seluruh halaman, setiap langkah | Hasil bertipe dari satu aksi, dibatasi 24 KB |
| Halaman yang berubah atau sudah keluar | Dibaca sebagai data | Terdeteksi dari respons yang dideklarasikan (`response_mismatch`, `auth_required`) |
| Perubahan di situs | Apa pun yang diputuskan model | Persetujuan sekali pakai untuk masukan yang persis, diberikan orang di terminal atau aplikasi agen |
| Teks di halaman yang berbunyi "abaikan instruksi sebelumnya" | Masuk ke prompt | Ditandai data tak tepercaya, tidak pernah menjadi alat |
| Kredensial | Diketik ke halaman oleh model | Di keyring OS; agen hanya melihat id akun |
| Saat diblokir | Coba lagi, ganti IP, tiru browser | Berhenti, memberi tahu Anda, menunggu Anda melanjutkan |
| Apa yang terjadi | Riwayat obrolan | Log audit berantai hash yang bisa Anda verifikasi |

## Yang bisa dan tidak bisa dilakukan

Setiap baris di sini benar untuk kode hari ini. `itda setup` menyebutkan kasus
mana yang ditemuinya; `itda draft page` atau `itda record` menyebutkan apa yang
mereka tinggalkan.

**Berfungsi sekarang**

- Halaman yang digambar di server: tabel, daftar dan kartu berulang menjadi
  rekaman, dan halaman yang berisi satu hal menjadi pembacaan atas
  bidang-bidangnya. Formulir pencarian menjadi pembacaan, formulir kirim menjadi
  perubahan, dan feed ikut serta.
- Halaman yang menyematkan datanya untuk skripnya (`__NEXT_DATA__`,
  `ld+json`, state Apollo atau state pramuat): satu pembacaan per daftar di
  dalamnya.
- Halaman yang digambar skrip yang memanggil API berjawaban JSON, misalnya
  portal React atau Vue: `itda setup` membuka browser Anda pada halaman itu dan
  mengamati apa yang dimuatnya dengan sendirinya. itda menyusun draf dari
  panggilan-panggilan itu. itda tidak mengklik atau mengetik.
- Formulir yang menginginkan token CSRF: langkah pendahuluan membaca token baru
  dari halaman formulir tepat sebelum setiap kiriman yang disetujui.
- Halaman yang hanya cangkang di sekeliling frame: isinya berada dalam
  `<iframe>` di situs yang sama, sering di balik proses masuknya sendiri. itda
  membaca setiap frame sebagai halaman situs dan masuk dahulu saat frame itu
  memintanya.
- Sesi cookie: masuk lewat formulir milik situs dengan nama pengguna dan kata
  sandi Anda, atau browser Anda sendiri untuk single sign-on dan kode sekali
  pakai. Sesi yang dibawa di alamat (`?sid=...`) bekerja dengan cara yang sama.
- Deskripsi OpenAPI atau Swagger yang diterbitkan situs, dan endpoint GraphQL
  yang menjawab introspeksi: satu pembacaan per operasi atau bidang kueri.
- Situs yang menjalankan GitLab, Jira, Confluence, Redmine, Discourse,
  MediaWiki, WordPress, Gitea atau Forgejo: API-nya sendiri, dari templat siap
  pakai.
- Apa pun yang Anda rekam sendiri saat memakainya, di browser Anda atau sebagai
  berkas HAR.

**Berfungsi setelah Anda rekam**

- Halaman yang tidak menampilkan apa pun pada pengambilan biasa dan tidak
  memuat apa pun dengan sendirinya. Ia baru mengambil data setelah Anda klik,
  cari atau masuk. `itda record` mengamati Anda memakainya dan menyusun draf
  dari panggilan yang dilihatnya.

**Tidak berfungsi**

- WebSocket. itda berbicara HTTP biasa, dan rekaman tidak menyimpan lalu lintas
  socket.
- API yang menandatangani setiap permintaan (tanda tangan atau nonce yang
  dihitung per panggilan). itda menyimpan token atau kunci API yang tetap di
  keyring. itda tidak mereproduksi nilai yang berubah pada setiap panggilan,
  jadi situs menolak permintaannya.
- Halaman yang dirakit dari potongan HTML (htmx dan sejenisnya). Bukan karena
  disengaja: itda mungkin masih membaca potongan yang berisi tabel utuh, tetapi
  tidak apa yang dilakukan skrip dengannya.
- Isi yang digambar di kanvas atau menjadi gambar. itda membaca teks dalam HTML,
  JSON dan XML; itda tidak melihat piksel.
- Situs di balik challenge atau pengelola bot (Cloudflare, Akamai, captcha).
  itda menyebut identitasnya dan tidak pernah meniru browser. itda menjeda situs
  itu sampai Anda melanjutkannya. Lihat
  [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md).
- Apa pun yang tidak bisa dilihat akun Anda. itda tidak punya akun sendiri; ia
  memakai akun Anda.

## Saat situs berubah

`itda check <site>` menjalankan setiap pembacaan yang tidak perlu masukan dan
tidak pernah mengirim perubahan. Ia berakhir dengan satu putusan: `ok`,
`signed_out`, `site_changed`, `blocked` atau `unreachable`. Kode keluar
mengikutinya (11 untuk `site_changed`), sehingga pekerjaan terjadwal bisa
memperingatkan Anda sebelum agen tersandung padanya. Lewat MCP, `itda_check`
mengembalikan hal yang sama.

Saat putusannya `site_changed`, `itda refresh <site>` menyusun ulang draf situs
dari sumber penyiapannya: halaman-halamannya, deskripsi API, endpoint GraphQL,
atau bentuk rekaman yang disimpan, dengan semua nilainya dikosongkan. Ia
menunjukkan apa yang akan berubah per aksi dan tidak menulis apa pun sampai
Anda bilang ya. itda mempertahankan aksi yang Anda sunting sendiri dan
memperbarui aksi yang diubah oleh draf baru. Aksi yang sudah tidak dihasilkan
sumbernya hanya dibuang jika pembacaannya juga gagal, dan setiap aksi baru
dicoba sekali, lalu ditambahkan jika menjawab. itda menarik persetujuan yang
menunggu untuk situs itu dan memeriksa situs lagi tepat sesudahnya. Definisi
yang dipasang dengan `itda add` tidak punya sumber dan tidak bisa disusun
ulang; sunting secara manual.

Itulah pertukarannya. Agen browser beradaptasi pada setiap jalannya, dan karena
itu lambat dan tidak pasti. itda adalah satu permintaan dan jawaban yang sama
setiap kali. Ia gagal dengan jelas saat situs berpindah.

## Berdampingan dengan agen browser

Playwright MCP, browser-use dan computer use mengendalikan browser pada setiap
tugas. itda tidak menggantikan mereka: halaman yang hanya Anda pakai sekali,
atau situs dari daftar di atas yang tidak dicakup itda, adalah bagian mereka.
itda untuk situs yang akan dipanggil agen berulang-ulang. Rekam sekali di
browser, tinjau aksinya, lalu jalankan lewat HTTP dengan jawaban bertipe,
persetujuan sekali pakai untuk setiap perubahan dan sebuah log. Pakai keduanya:
agen browser untuk menjelajah, itda untuk panggilan yang berjalan setiap hari.

## Mulai cepat

```sh
# macOS dan Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
```

```powershell
# Windows
powershell -ExecutionPolicy Bypass -c "irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex"
```

Unduh arsip dari [Releases](https://github.com/brnyxx/itda-cli/releases), atau
bangun dari sumber dengan
`cargo install --git https://github.com/brnyxx/itda-cli itda-cli`.

Claude Desktop, tanpa terminal: unduh bundel untuk
[macOS (Apple silicon)](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-aarch64-apple-darwin.mcpb),
[macOS (Intel)](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-x86_64-apple-darwin.mcpb) atau
[Windows](https://github.com/brnyxx/itda-cli/releases/latest/download/itda-x86_64-pc-windows-msvc.mcpb) lalu buka berkasnya. Claude
Desktop memasang itda beserta alat-alatnya, dan "pakai the-site-you-use.example"
menyiapkan sebuah situs dari dalam percakapan. Menyetujui perubahan tetap
dilakukan di terminal dengan `itda approve`, jadi untuk perubahan pasang juga
perintahnya.

Ketik `itda` dan alamat situs yang Anda pakai. itda menemukan aplikasi agen di
komputer Anda dan menghubungkannya setelah satu Enter. itda berbicara dua belas bahasa:
`LANG` memilih satu dan `ITDA_LANG` menggantikannya. `itda` saja meminta alamat; `itda setup <address>`
menerima flag untuk skrip.

itda mengenali situs yang menjalankan perangkat lunak yang dikenal. Saat halaman
menyatakan dirinya GitLab, Jira, Confluence, Redmine, Discourse, MediaWiki,
WordPress atau Gitea (juga Forgejo), dan satu permintaan ke API perangkat lunak
itu sendiri memastikannya, agen mendapat API asli perangkat lunak tersebut dari
templat siap pakai ([`sites/software/`](sites/software/)), dengan cara masuk
yang dipakai perangkat lunak itu. Apa yang ditampilkan halaman itu sendiri ikut
bergabung. `itda setup --as jira` memaksa sebuah templat; `--no-template`
membaca halaman seperti halaman lainnya.

Halaman yang digambar skrip hanya menunjukkan judul kepada itda, jadi itda
membuka browser Anda padanya selama beberapa detik. itda mengamati apa yang
dimuat halaman dengan sendirinya dan menyusun draf dari panggilan-panggilan itu.
Tidak ada yang diklik atau diketik, dan jendelanya menutup sendiri. Hanya
halaman yang tetap tidak menampilkan apa pun, atau perlu masuk, yang meminta
Anda memakai situs di browser sementara itda mengamati.

Setelah agen terhubung, Anda juga bisa langsung memintanya: "siapkan
news.ycombinator.com untuk saya". Agen memanggil `itda_setup`. itda melihat
situs, menyusun draf dan mencobanya dengan cara yang sama. Aksi-aksinya muncul
sebagai alat dalam percakapan yang sama. Halaman berskrip terbuka sebentar di
jendela browser pada layar Anda. Saat sebuah pembacaan minta masuk,
`itda_login` membuka browser Anda sendiri; agen hanya melihat id akun.

Lewati terminal dan serahkan ini ke Claude Code, Cursor atau Codex, bersama
alamat situs yang Anda pakai:

```text
Pasang itda dan siapkan the-site-you-use.example untuk saya.
1. Pasang: curl -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
   (di Windows: irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex)
2. Jalankan: itda setup https://the-site-you-use.example --client claude-code -y
   (pakai cursor, codex, atau claude-desktop jika itu dirimu)
3. Baca https://github.com/brnyxx/itda-cli/blob/main/AGENTS.md, lalu pakai situs itu lewat itda.
   Pembacaan langsung jalan. Apa pun yang mengubah situs menunggu persetujuan saya.
   Jika sebuah pembacaan minta masuk, beri tahu saya untuk menjalankan: itda login <site> --browser
```

Bagian-bagiannya adalah perintah tersendiri. Untuk halaman yang bisa Anda buka
tanpa masuk, beri itda alamatnya. Tabel, daftar dan kartu berulang menjadi
rekaman. Halaman yang berisi satu hal (paket, artikel, produk) menjadi
pembacaan atas judul, isi dan nilai berlabelnya. Daftar yang tautannya berbagi
satu bentuk mendapat pembacaan satu rekaman berdasarkan id-nya. Formulir
pencarian menjadi pembacaan; formulir kirim menjadi perubahan. Feed ikut serta.
Endpoint GraphQL yang disebut halaman ditanya sekali apa yang ditawarkannya,
satu pembacaan per bidang kueri. Saat situs menerbitkan deskripsi API,
pembacaan-pembacaannya bergabung ke draf:

```sh
itda draft page https://example.com/board --install   # menunjukkan apa yang ditemukan di halaman
itda call example.get-board --page 2 --json
itda check example                                     # apakah halaman masih cocok?
```

Untuk situs di balik formulir masuk, beri `itda setup` halaman dan nama
pengguna Anda. itda mengirim formulir milik situs itu sekali, sebagai dirinya
sendiri, dengan kata sandi yang Anda ketik. itda menyimpan sesi di keyring
sistem dan menyusun draf dari halaman seperti yang Anda lihat setelah masuk.
Definisi mengingat formulirnya, sehingga `itda login board` masuk lagi saat
sesi kedaluwarsa:

```sh
itda setup https://intranet.example/board --name board --user me   # meminta kata sandi
itda call board.list-notices --json                 # pembacaan langsung jalan
itda call board.create-notice --title "Server maintenance"   # perubahan menunggu persetujuan Anda
itda login board --user me                          # saat sesi kedaluwarsa
```

Untuk single sign-on atau kode sekali pakai, rekam diri Anda memakai situs
sebagai gantinya. itda membuka browser Anda sendiri di jendela baru. itda
mengamati apa yang diminta halaman dari server sementara Anda masuk, mencari
dan mengirim formulir; itda tidak pernah mengklik atau mengetik:

```sh
itda record https://intranet.example/board --name board --install   # tekan Enter saat selesai
                                                    # lalu jawab ya untuk menyimpan sesi atau token yang dipakainya
itda mcp install claude-desktop                     # atau biarkan agen Anda memanggilnya
```

`examples/lab` menjalankan Gitea, Redmine, WordPress, MediaWiki, sebuah intranet
lama kecil dan tiga kasus yang lebih jelek di balik proses masuknya: papan di
dalam iframe, tabel yang digambar skrip dari XHR, dan sesi yang dibawa di URL.
Sebuah rangkaian uji menelusuri masing-masing dari penyiapan sampai perubahan
yang disetujui.

Rekaman tetap di memori dan hanya bentuk setiap panggilan yang mencapai draf:
tidak ada header, cookie atau nilai yang Anda ketik. Berkas HAR yang disimpan
dari developer tools browser bekerja dengan cara yang sama lewat
`itda draft har`.

API publik tidak perlu rekaman. Situs siap pakai berikut dari
[sites/](sites/README.md) sudah tertanam dan tidak perlu akun:

<p><a href="sites/github.yaml"><img src="assets/brands/github.svg" width="36" height="36" alt="GitHub"></a> <a href="sites/npm.yaml"><img src="assets/brands/npm.svg" width="36" height="36" alt="npm"></a> <a href="sites/crates-io.yaml"><img src="assets/brands/crates-io.svg" width="36" height="36" alt="crates.io"></a> <a href="sites/pypi.yaml"><img src="assets/brands/pypi.svg" width="36" height="36" alt="PyPI"></a></p>

```sh
itda add github      # atau npm, crates-io, pypi
itda call github.repo --owner rust-lang --repo rust
```

Coba sandbox yang disertakan, intranet palsu yang hanya mendengarkan di
127.0.0.1: ikuti [examples/sandbox](examples/sandbox/README.md). Definisi situs
adalah satu berkas YAML. `itda add spec.yaml` memasangnya setelah menunjukkan ke
mana ia mengirim permintaan dan aksi mana yang mengubah situs. `itda doctor`
mencetak lokasi direktori data. [docs/SPEC.md](docs/SPEC.md) menjelaskan cara
menulis definisi.

```sh
itda setup <url>                         # halaman atau rekaman -> aksi bernama -> dicoba -> agen, dengan pertanyaan
itda draft page <url>... [--install]     # susun draf situs dari halaman yang Anda pakai
itda record <url> [--install]            # atau dari memakainya di browser Anda, di balik proses masuk
itda draft har <file> / openapi <file>   # atau dari rekaman tersimpan atau deskripsi API
itda add spec.yaml / remove <site>       # pasang atau hapus situs (juga URL atau github)
itda ls                                  # situs terpasang dan aksinya
itda describe <site>                     # masukannya dan perintah untuk disalin per aksi
itda call <site>.<action> --input value  # jalankan satu; --json untuk agen
itda login <site> --user me              # masuk lewat formulir milik situs (kata sandi dari stdin atau prompt)
echo "$KEY" | itda login <site> --secret api-key   # atau simpan kunci API di keyring OS
itda login <site> --browser              # atau masuk di browser Anda sendiri, sekali (tersegel di bawah direktori data)
itda accounts / logout <site>            # lihat atau hapus proses masuk tersimpan
itda approvals / approve <id> / reject <id>
itda audit [--verify]                    # apa yang dilakukan agen
itda check [<site>|--all]                # apakah setiap pembacaan masih cocok dengan situs?
itda refresh <site>                      # situs berubah: susun ulang draf, pertahankan yang Anda putuskan
itda resume <site>                       # setelah situs menolak atau kunci kedaluwarsa
itda feedback                            # laporkan masalah: membuka isu baru yang sudah terisi
```

## Pakai dari agen

```sh
itda mcp install claude-desktop   # atau cursor, vscode, codex
itda mcp install claude-code      # mencetak perintah `claude mcp add`
# tambahkan --approve-in-client untuk menyetujui perubahan di formulir klien sendiri
# itda mcp --site <name> melayani satu situs saja, untuk konfigurasi milik proyek
```

Pemasang menggabungkan ke konfigurasi klien dan mempertahankan server Anda yang
lain. Ia mencadangkan berkas lama sebagai `.bak`. `--dry-run` menunjukkan
perubahannya dahulu. `itda mcp` melayani setiap situs yang terpasang:

- Sampai 30 aksi muncul sebagai satu alat masing-masing (`sandbox.list-notices`),
  dengan skema masukan serta petunjuk hanya-baca, destruktif dan idempoten.
- Lebih dari itu, tiga alat tetap (`itda_search`, `itda_describe`, `itda_call`)
  menjaga daftar alat tetap pendek dan cache prompt tetap sah. `--mode`
  menimpa ini.
- Hasilnya adalah amplop JSON yang sama dengan `itda call --json`, ditandai tak
  tepercaya.
- Perubahan mengembalikan `approval_required` dengan sebuah id. Tidak ada alat
  yang menyetujui; agen harus meminta Anda.

Agen yang hanya punya shell bisa memanggil `itda call … --json` dan bercabang
pada kode keluar:

| Keluar | Arti |
|---|---|
| 0 | ok |
| 2 | masukan buruk atau aksi tidak dikenal |
| 10 | masuk lagi (`itda login`) |
| 11 | halaman sudah tidak cocok dengan definisinya |
| 12 | situs menolak; dijeda sampai `itda resume` |
| 13 | dibatasi lajunya |
| 14 | situs menolak permintaan yang satu ini sebagai tidak diizinkan; masuk lagi tidak akan membantu |
| 20 | perlu persetujuan, atau pengguna belum menjawab (atau perubahan dimatikan) |
| 21 | persetujuan sudah dipakai, ditolak, kedaluwarsa atau ditarik karena masukan berubah |
| 22 | itda mengirim perubahan tetapi hasilnya tidak diketahui; dengan `outcome_check`, pembacaan yang disebut perubahan itu menyatakan apakah perubahan sudah masuk |
| 30 | galat jaringan |

## Prinsip

- **Lokal dahulu.** Definisi berjalan di perangkat Anda. Kredensial tidak pernah
  meninggalkannya.
- **Hanya-baca secara bawaan.** Sebuah aksi adalah pembacaan hanya jika metode,
  nama, path, masukan dan dokumen GraphQL-nya semua sepakat. Selain itu adalah
  perubahan.
- **Orang menyetujui perubahan.** Setiap persetujuan mencakup satu permintaan
  dengan masukan, akun, definisi dan kredensial yang persis. Ia berlaku sekali
  dan kedaluwarsa.
- **Pemblokiran dilaporkan, tidak diakali.** itda menyebut identitasnya dan
  tidak pernah meniru browser. Ia menjeda situs yang menolaknya. Lihat
  [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md).
- **Aksi terverifikasi.** Sebuah definisi menyatakan seperti apa respons yang
  benar, sehingga "sudah keluar" dan "situs berubah" adalah galat, bukan data.

## Tata letak

Crate hanya bergantung pada crate di bawahnya; `cargo xtask check-layers`
menegakkannya.

| Lapisan | Crate | Tujuan |
|---|---|---|
| 0 | `itda-types` | Kontrak galat, pemeriksaan URL (Public Suffix List), penyuntingan |
| 1 | `itda-html`, `itda-store` | Ekstraksi HTML bebas skrip; penyimpanan SQLite lokal |
| 2 | `itda-spec` | Format definisi, pembangunan permintaan, verifikasi respons |
| 3 | `itda-policy` | Klasifikasi baca/tulis, pemeriksaan keamanan definisi, persetujuan |
| 4 | `itda-net` | HTTP terbatas, pengaturan laju, deteksi pemblokiran, penjaga alamat privat |
| 5 | `itda-vault` | Akun dan rahasia di keyring OS atau lingkungan |
| 7 | `itda-runtime` | Menjalankan aksi: kredensial, persetujuan, pembentukan, log audit |
| 9 | `itda-setup` | Langkah penyiapan, masuk dan pemeriksaan, untuk CLI dan server MCP sekaligus |
| 10 | `itda-mcp` | Server MCP lewat stdio |
| 11 | `apps/cli` | Perintah `itda` |

## Mengembangkan

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo xtask check-layers
```

`rust-toolchain.toml` menyematkan Rust 1.98 (edisi 2024). Uji hanya berbicara
dengan server lokal yang mereka jalankan sendiri. Aturan commit dan PR ada di
[CONTRIBUTING.md](CONTRIBUTING.md).

## Keamanan

[docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) mendaftar apa yang dipertahankan
itda dan apa yang tidak. Laporkan kerentanan seperti dijelaskan di
[SECURITY.md](SECURITY.md).

## Kredit dan lisensi

Berlisensi [Apache-2.0](LICENSE). Sebagian proyek ini diturunkan dari
[Hycli](https://github.com/Hybirdss/Hycli) (Apache-2.0); lihat [NOTICE](NOTICE).
Setiap berkas turunan menyatakannya di header dan menjelaskan apa yang berubah.
Proyek ini tidak berafiliasi dengan Hycli dan tidak memakai nama atau mereknya.
