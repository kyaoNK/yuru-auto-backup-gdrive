# yuru-auto-backup-gdrive 設計書

## 1. 概要

Premiere Pro のプロジェクトファイル (`.prproj`) を、毎日 1 回の定時ジョブで Google Drive デスクトップクライアントが同期済みのローカルフォルダへ自動コピーする Windows 向けデスクトップアプリ。Google Drive クライアントが同期を肩代わりすることで、結果として「クラウド自動バックアップ」を実現する。

本アプリは、既存の PowerShell スクリプト（`backup_prproj.ps1` + `register_task.ps1`）を GUI 化し、パス設定・実行時刻の変更・動作状況の確認を非エンジニアでも容易に行えるようにしたもの。

## 2. 目的 / 背景

- Premiere Pro の編集用 PC が故障しても、他メンバーが `.prproj` をクラウドから取得して作業を引き継げるようにする。
- 既存の PowerShell スクリプトは「メモ帳に貼り付けてパスを書き換えて右クリック実行」という運用負荷があるため、GUI で完結させる。
- 既存スクリプトの運用ルール（対象／除外／命名規則／上書き）を踏襲する。

## 3. 動作環境

| 項目 | 内容 |
| --- | --- |
| OS | Windows 11 |
| フレームワーク | Tauri (v2) |
| フロントエンド | TypeScript + Svelte |
| バックエンド | Rust |
| 前提ソフトウェア | Google Drive for desktop（同期クライアント） |

## 4. バックアップ仕様（既存スクリプト踏襲）

### 4.1 対象ファイル

- 拡張子: `.prproj` のみ
- 監視元フォルダ配下を**再帰的**に探索
- フォルダ名条件: 祖先パスのいずれかのフォルダ名が正規表現 `^\d{6}\(` にマッチする必要がある（例: `250304(3)_Project`）
  - 既存スクリプトの `FullName -match "\\\d{6}\("` に相当

### 4.2 除外条件

- パスに `Auto-Save` を含むもの（Premiere Pro 自動保存）を常に除外（ハードコード）
- 加えて、以下の**ユーザー設定による追加除外**を適用する（どちらも空でも可）:
  - `excludedFolders`: 絶対パスのリスト。ファイルの祖先パスのいずれかがこのリストのいずれかと一致すれば除外（= そのフォルダ配下のサブツリーを丸ごと除外）
  - `excludedFolderNames`: フォルダ名のリスト。ファイルの祖先フォルダ名のいずれかがこのリストのいずれか（大文字小文字を区別しない）と一致すれば除外（例: `Proxy`, `Cache`）
- 上記のいずれか 1 つにでも該当したファイルは対象外。ハードコードされた `Auto-Save` 除外と `^\d{6}\(` 必須条件は**そのまま固定**で、これらと独立・追加で適用される。

### 4.3 出力先・命名規則

- 出力先: **単一のフォルダ**（サブフォルダ構造は作らない、平置きコピー）
- ファイル名: `<元ファイル名の BaseName>_Latest.prproj`
  - 例: `250304(3)_クイズ.prproj` → `250304(3)_クイズ_Latest.prproj`
- 既存ファイルは**常に上書き**（履歴は残さない）

### 4.4 実行タイミング

**保持期限（2026-09-30 追加）**: 元ファイルの最終更新日時から暦の **2ヶ月**（60日固定ではなく、月末は到達月の末日に丸める）経過したファイルはコピー対象外とし、対応する管理済みバックアップを定時／手動実行時に削除する。元ファイルは削除しない。再編集で期限内になればコピーを再開する。期限はソース固定で設定項目は増やさない。

安全のため出力先の `.yuru-backup-manifest.json` に、この機能導入後にコピー成功したファイルの元パス・サイズ・更新日時・SHA-256 を記録する。削除は、現行の対象／除外条件に合う元ファイルが存在し、対応する管理記録と出力先のサイズ・更新日時・SHA-256 が一致する通常ファイルだけに限定する。未管理の既存バックアップ、元ファイル不在、除外済み、外部変更されたバックアップは削除しない。ハッシュのない旧管理記録は削除の根拠にせず、期限内の再コピーで更新する。削除直前に元ファイルの更新日時を再確認する。管理記録が破損していた場合はジョブをエラー終了する。削除件数・期限切れ件数はログに記録し、削除失敗はエラー件数に含める。Google Drive にも削除が同期される。

- 既定: 毎日 **09:00**（変更可能）
- 実行時、出力先フォルダが見えるまで最大 **5 分**待機（Google Drive for desktop の起動を待つ）
- 出力先が 5 分経っても見えなければ今回の実行は中止してログに記録
- PC がオフだった場合: 次回起動時に取りこぼしを実行（Windows の `StartWhenAvailable` 相当）

### 4.5 起動方式

常駐方式。アプリが Scheduler を内包し、Windows ログオン時に自動起動して指定時刻に BackupJob を発火させる。

## 5. 機能要件

| ID | 機能 | 概要 |
| --- | --- | --- |
| F-01 | 監視元フォルダ設定 | UI でフォルダ選択ダイアログから指定できる |
| F-02 | 出力先フォルダ設定 | Google Drive 同期済みのローカルフォルダを指定できる（自動検出あり） |
| F-03 | 実行時刻設定 | 毎日 1 回の実行時刻（HH:MM）を変更できる（既定 09:00） |
| F-04 | 定時ジョブ実行 | 指定時刻に `.prproj` をスキャンしてコピーする |
| F-05 | 取りこぼし補填 | アプリ起動時、本日の実行時刻を過ぎていて未実行なら即時実行（常時有効） |
| F-06 | 手動実行 | UI から「今すぐ実行」できる |
| F-07 | Drive 同期待機 | 出力先が現れるまで最大 5 分リトライ |
| F-08 | ステータス表示 | 稼働状態 / 最終実行時刻 / 次回実行時刻 / 直近結果を表示 |
| F-09 | ログ表示 | コピー／エラー件数と対象ファイル一覧を閲覧できる |
| F-10 | システムトレイ常駐 | ウィンドウを閉じても常駐し、トレイから手動実行と設定に飛べる |
| F-11 | 自動起動 | Windows ログオン時に自動起動（ON/OFF 可） |

## 6. 非機能要件

- **常駐メモリ**: アイドル時 80MB 未満を目標（スケジュール待機が主）。
- **CPU**: アイドル時はほぼ 0%。ジョブ実行中も 1 コアを使い切らない。
- **信頼性**: コピー中にファイルが破損しないよう一時ファイル `*.part` → rename の原子的置換を使用。
- **再起動耐性**: 設定・「最終実行時刻」をディスクに永続化し、OS 再起動後もスケジュール状態を復元する。

## 7. システム構成

```
┌──────────────────────────── Tauri App ────────────────────────────┐
│                                                                    │
│  [ Frontend (WebView) ]          [ Backend (Rust) ]                │
│   ├─ 設定画面                     ├─ Scheduler (定時トリガ)        │
│   ├─ ステータス表示               ├─ BackupJob                     │
│   ├─ ログ表示                     │   (スキャン＋フィルタ＋コピー)  │
│   └─ Tauri Command 呼び出し ────▶ ├─ DriveWaiter (最大5分待機)    │
│                                   ├─ DrivePathDetector             │
│                                   ├─ ConfigStore (JSON 永続化)     │
│                                   ├─ Logger                        │
│                                   └─ Tray / Autostart              │
└────────────────────────────────────────────────────────────────────┘
                │ ファイルコピー
                ▼
   [ Google Drive 同期フォルダ (ローカル) ]
                │ Google Drive for desktop が自動アップロード
                ▼
         [ Google Drive (Cloud) ]
```

## 8. 主要モジュール

### 8.1 Scheduler
- 専用スレッド＋チャネルで、ローカル TZ の次回期限を保持し、最大30秒ごとに実時刻を確認して発火する。スリープ復帰・時計変更後も残りの相対待機時間に拘束されない。UI は Scheduler が保持する実際の次回期限を参照する。
- ジョブは同じ専用スレッドで直列実行。手動要求の受付から完了まで排他フラグを保持し、重複要求を受け付けない。不正な時刻・パス設定では定時／手動とも実行しない。
- **取りこぼし補填**: アプリ起動時、`lastRunAt` が本日でなく、かつ本日の実行時刻を過ぎていれば即時実行。

### 8.2 DriveWaiter
- 出力先がディレクトリであることを10秒間隔で確認、最大5分（300秒）リトライ。残時間を超えて待機しない。終了要求で待機を中断可能。
- タイムアウトしたら `Err(DriveNotReady)` を返し、BackupJob は中止してログに記録。
- （既存スクリプトの `Test-Path $DEST` ループと同等）

### 8.3 固定フィルタ定数（ハードコード）

```rust
const TARGET_EXTENSION: &str = "prproj";
const EXCLUDE_PATH_KEYWORD: &str = "Auto-Save";
const FOLDER_NAME_REGEX: &str = r"^\d{6}\(";
const DRIVE_WAIT_SECONDS: u64 = 300;
const BACKUP_SUFFIX: &str = "_Latest.prproj";
const RETENTION_MONTHS: u32 = 2;
```

既存スクリプトの運用ルールはユーザーが設定で変える意味がないため、定数として固定する。将来変更したい場合はソース改修で対応。

### 8.4 DrivePathDetector

Google Drive for desktop の同期ルート（例: `G:\My Drive`、`G:\Shared drives`）を自動検出する。出力先フォルダ選択時に、検出されたルートを起点にフォルダ選択ダイアログを開くことで、非エンジニアでも迷わず目的のフォルダに辿り着ける。

**実装済みの検出（複数ソースの候補を統合）:**

1. レジストリ `HKCU\Software\Google\DriveFS` 配下のパスから、既知の `My Drive` / `Shared drives` / 日本語名のディレクトリを探す。
2. ドライブレター A〜Z 配下にある同じ既知名のディレクトリを探す。
3. `%USERPROFILE%\Google Drive` を確認する。
4. 存在する候補だけを、正規化したパス（Windows では大小文字無視）で重複排除する。

非公開の Drive 設定 DB 解析・ボリュームラベル判定は未実装。候補は推測であり、クラウド同期の完了／有効性を保証しない。

検出結果は候補リストとして返し、UI では候補のラベルとパスを表示する。検出ゼロ件の場合はユーザーに「Google Drive for desktop が起動しているか確認してください」と案内し、通常のフォルダ選択ダイアログにフォールバックする。

バージョンアップで内部構造が変わって壊れる可能性があるため、**検出失敗は致命エラーにせず、常に手動選択へフォールバック可能**にする。

### 8.5 BackupJob
擬似コード:
```rust
fn run(cfg: &Config) -> JobSummary {
    DriveWaiter::wait(&cfg.destination, Duration::from_secs(DRIVE_WAIT_SECONDS))?;

    let folder_re = Regex::new(FOLDER_NAME_REGEX).unwrap();
    let mut summary = JobSummary::default();

    for entry in walkdir(&cfg.source) {
        if entry.extension() != Some(TARGET_EXTENSION) { continue; }
        if entry.path().contains(EXCLUDE_PATH_KEYWORD) { continue; }
        if !ancestor_matches(&entry, &folder_re) { continue; }

        let dest_name = format!("{}{}", entry.file_stem(), BACKUP_SUFFIX);
        let dest = cfg.destination.join(dest_name);

        match copy_atomic(entry.path(), &dest) {
            Ok(_)  => summary.copied += 1,
            Err(e) => { summary.errors += 1; log::warn!(...); }
        }
    }
    summary
}
```
- `copy_atomic`: 出力先と同じディレクトリに排他的な一意の `.part` を作成し、コピー・内容照合・sync 後に原子的に置換する。Windows では元ファイルの書込／置換を読み取り中拒否し、使用中・変化検知時は旧バックアップを残す。元と出力先が同一の設定を拒否し、除外サブツリー・リンクは走査しない。
- **シリアル実行**（`.prproj` は小さく本数も少ないため並列化不要）。
- 完了時に `lastRunAt`, `JobSummary`（`copied` / `errors` の 2 指標）を ConfigStore / Logger に反映。
- 未設定・Drive 待機タイムアウト・ジョブ中断時は `lastError`（`at` / `message`）を保存し、再起動後もダッシュボードに失敗日時・理由を表示する。`lastRunAt` / `lastSummary` は前回完了した実行の値を維持し、取りこぼし判定は変更しない。次のジョブが完了したら `lastError` を解除し、部分失敗は `lastSummary.errors` で明示する。設定保存では実行状態を上書きしない。実行結果の保存に失敗した場合も成功通知ではなくエラー通知する。

### 8.6 ConfigStore

**保存先の決定ロジック** (`AppDir::resolve()`):

1. 既定インストール先（LocalAppData / Program Files 配下の製品フォルダ）の場合、`%USERPROFILE%\yuru-auto-backup-gdrive\` を使用し、旧 `data/` の状態があれば未存在ファイルだけを原子的に移行する。
2. 管理インストール以外で実行ファイル隣に既存の状態がある場合はその `data/` を優先する。書込不可なら別設定へ黙って切り替えず起動エラーを表示。
3. それ以外では既存のホーム側状態を優先し、なければ書込可能な `data/`、最後にホームへフォールバックする。

書込試験も一意の一時ファイルで行う。主設定が壊れた／不在なら正常な `.bak` を読み、両方から復旧不能なら既定値に戻さずエラーを表示する。保存時は正常な主ファイルだけを `.bak` に保全し、壊れた主ファイルで正常なバックアップを上書きしない。設定・管理記録の更新は sync 済みの一時ファイルから原子的に置換する。

どちらの場合も、配下に以下のファイル/ディレクトリを置く:

```
<AppDir>/
├─ config.json
└─ logs/
   └─ backup.log
```

**設定ファイル (`config.json`) スキーマ**:

```json
{
  "source": "D:/PremiereProjects",
  "destination": "G:/Shared drives/Team/backup",
  "scheduleTime": "09:00",
  "autoStart": true,
  "excludedFolders": ["D:/PremiereProjects/250304(3)_Project/Proxy"],
  "excludedFolderNames": ["Cache", "Render"],
  "lastRunAt": "2026-04-23T09:00:12+09:00",
  "lastSummary": { "copied": 3, "errors": 0 },
  "lastError": null
}
```

9 項目（編集可能な6項目＋実行状態3項目）。`lastError` は旧設定では未指定でも読み込める。ハードコードされた `Auto-Save` 除外と `^\d{6}\(` 必須は 8.3 の定数で維持しつつ、ユーザーが現場の運用で追加したい除外（例: `Proxy` / `Cache` / `Render`）はここで持つ。挙動を規定する他のパラメータ（対象拡張子・Drive 待機秒数・`_Latest.prproj` サフィックス）は 8.3 の定数のまま変更不可。

### 8.7 Logger
- `<AppDir>/logs/backup.log` へ追記。1 日 1 ジョブなのでローテーションは行わず単一ファイル（必要に応じて手動削除）。
- ファイル末尾から読み取り、UI では新しい順に表示する。API の上限は5000件／1MiB、通常画面は500件。巨大な単一行では末尾部分のみとなる。メッセージ内の改行はエスケープし、ログ書込失敗は画面に通知する。
- 各ジョブの先頭に開始ヘッダ、末尾にサマリ、途中にファイル毎の結果を記録。

## 9. UI 設計

### 9.1 画面構成
1. **ダッシュボード**
   - 稼働状態（次回 09:00 / 実行中 / エラー）
   - 最終実行時刻と結果サマリ（コピー N 件 / エラー M 件）
   - 次回実行予定時刻
   - 「今すぐ実行」ボタン
2. **設定**
   - 監視元フォルダ（フォルダ選択ダイアログ）
   - 出力先フォルダ
     - 「Google Drive を検出」ボタン: `DrivePathDetector` が検出した候補を一覧表示し、選択するとそのルートを起点にフォルダ選択ダイアログを開く。
     - 「手動で選ぶ」ボタン: 通常のフォルダ選択ダイアログを開く（検出に失敗した場合のフォールバック）。
     - 選択後は絶対パスと「Drive 同期フォルダ配下かどうか」を表示して確認を促す。
   - 実行時刻（HH:MM スピナ、既定 09:00）
   - 自動起動 ON/OFF
3. **ログ**: 新しい順の時系列表示、レベル別フィルタ、再読み込み・取得失敗の表示。ジョブ別折りたたみは未実装。

### 9.2 トレイメニュー
- 状態（● 次回 HH:MM / ⏳ 実行中 / ⚠ エラー）
- 今すぐ実行
- 設定を開く
- 終了

## 10. 処理フロー

### 10.1 起動時
1. `AppDir::resolve()` で設定ディレクトリを決定（実行ファイル隣の `data/` → ダメなら `%USERPROFILE%\yuru-auto-backup-gdrive\`）。
2. ConfigStore 読み込み。
3. `source` と `destination` の妥当性チェック（未設定／存在しない場合は UI に警告）。
4. 取りこぼし判定: 本日の `scheduleTime` を過ぎていて `lastRunAt` が本日でなければ即時 BackupJob。
5. Scheduler 起動（次回実行時刻まで待機）。
6. トレイアイコン表示。

### 10.2 定時発火時
```
Scheduler 発火
  → DriveWaiter で destination が見えるまで最大 5 分リトライ
      ├─ タイムアウト: ログに記録して終了（次回まで待機）
      └─ 可視: BackupJob 開始
          → source を再帰スキャン → 拡張子／Auto-Save／フォルダ名正規表現でフィルタ
          → 各対象: <BaseName>_Latest.prproj として原子的コピー（上書き）
          → 完了: lastRunAt / lastSummary 更新、ログ追記、UI 通知
```

### 10.3 手動実行
- 「今すぐ実行」は Scheduler に手動要求を送信。次回定時期限は変えない。受付済み／実行中の重複要求は拒否し、受付の成否を UI に返す。

### 10.4 終了時
- 新規受付を停止し、Drive 待機はキャンセル。進行中のコピー等の完了は UI とは別スレッドで最大30秒待機してプロセス終了。起動時の保存先・移行・ログ初期化エラーはダイアログで通知する。

## 11. Tauri Command インターフェース

| Command | 入出力 | 用途 |
| --- | --- | --- |
| `get_config` | `() -> Config` | 設定取得 |
| `update_config` | `(Config) -> Result<()>` | 設定更新（Scheduler に即時反映） |
| `pick_folder` | `(start_dir?: PathBuf) -> Option<PathBuf>` | フォルダ選択ダイアログ（起点ディレクトリを任意で指定） |
| `detect_drive_roots` | `() -> Vec<DriveCandidate>` | Google Drive 同期ルート候補を返す |
| `get_status` | `() -> Status` | 稼働状態 / 最終実行時刻 / 次回実行時刻 / 直近サマリ |
| `run_now` | `() -> Result<bool>` | 手動バックアップ実行 |
| `preview_deletions` | `() -> DeletionPreview` | 削除予定・保護対象・確認エラーの読み取り専用プレビュー |
| `list_recent_logs` | `(limit) -> Vec<String>` | 直近ログ取得 |
| `open_app_dir` | `() -> Result<()>` | 設定・ログが置かれているディレクトリをエクスプローラで開く |

イベント（Rust → フロント）: `status-changed`, `job-started`, `job-finished`, `error-occurred`。

## 12. ディレクトリ構成（想定）

**ソースコード側**:
```
yuru-auto-backup-gdrive/
├─ src-tauri/
│  ├─ src/
│  │  ├─ main.rs
│  │  ├─ app_dir.rs       # 設定・ログの保存先決定
│  │  ├─ scheduler.rs
│  │  ├─ backup.rs        # BackupJob + 固定フィルタ定数
│  │  ├─ drive_waiter.rs
│  │  ├─ drive_detector.rs # Google Drive 同期ルートの自動検出
│  │  ├─ config.rs
│  │  ├─ logger.rs
│  │  └─ commands.rs
│  └─ tauri.conf.json
├─ src/
│  ├─ routes/
│  ├─ components/
│  └─ lib/
├─ package.json
└─ DESIGN.md
```

**インストール後のランタイム側**（ユーザー環境）:
```
<インストール先>/
├─ yuru-auto-backup-gdrive.exe

（インストーラー版は更新時の削除を避けるため以下を使用）
%USERPROFILE%\yuru-auto-backup-gdrive\
├─ config.json
└─ logs/backup.log

（手動配置のポータブル版のみ）
<実行ファイルと同じディレクトリ>/data/
├─ config.json
└─ logs/backup.log
```

## 13. 主要依存クレート / ライブラリ

- Rust: `tauri`, `serde` + `serde_json`, `tokio`, `walkdir`, `regex`, `chrono`, `tauri-plugin-autostart`, `tauri-plugin-dialog`, `tauri-plugin-opener`
- フロント: `@tauri-apps/api`, Svelte 5 / SvelteKit / Tailwind CSS

## 14. 実装ステップ

1. Tauri プロジェクト雛形を作成。
2. `AppDir::resolve()` を実装（実行ファイル隣の `data/` → `%USERPROFILE%` フォールバック）。
3. ConfigStore を実装（JSON 読み書き）＋ 設定 UI（監視元 / 出力先 / 実行時刻 / 自動起動）。
4. BackupJob を実装（`walkdir` + 固定フィルタ + `_Latest.prproj` 命名 + 原子的コピー）。
5. DriveWaiter を実装（最大 5 分ポーリング）。
6. DrivePathDetector を実装（8.4 のレジストリ / ドライブレター / 慣習パスの候補統合。内部 DB 解析は未実装）。
7. Scheduler を実装（専用スレッド・実時刻確認 + 取りこぼし補填）。
8. Tauri Command / イベント経由で UI から「今すぐ実行」と進捗表示を実装。設定画面に Drive 検出ボタンを組み込む。
9. Logger とログ画面、ダッシュボードのサマリ表示を実装。
10. トレイ常駐・自動起動（`tauri-plugin-autostart`）を組み込み。
11. 動作確認:
    - `.prproj` 以外が除外されること
    - `Auto-Save` 配下が除外されること
    - `^\d{6}\(` 条件を満たすフォルダ配下のみコピーされること
    - 出力ファイル名が `_Latest.prproj` で上書きされること
    - Google Drive 停止状態で起動 → 5 分以内に起動した場合にコピーされること
    - PC スリープ後の起動で取りこぼしが実行されること
    - Drive 検出ボタンを押すとマウント済みの Google Drive ルートが候補に出ること（レジストリ／設定ファイル／慣習パスの各経路）
    - 実行ファイル隣の `data/` に書けない環境で `%USERPROFILE%` にフォールバックすること

## 15. 既存 PowerShell スクリプトとの対応表

| 既存スクリプト | 本アプリ |
| --- | --- |
| `$SRC` | `config.source`（UI で選択） |
| `$DEST` | `config.destination`（UI で選択、Drive 自動検出あり） |
| `while (!(Test-Path $DEST) ...)` | `DriveWaiter`（`DRIVE_WAIT_SECONDS=300` で固定） |
| `Get-ChildItem -Filter *.prproj -Recurse` | `walkdir` + `TARGET_EXTENSION="prproj"` |
| `$_.FullName -notmatch "Auto-Save"` | `EXCLUDE_PATH_KEYWORD="Auto-Save"`（ハードコード） + `config.excludedFolders` / `config.excludedFolderNames`（ユーザー設定） |
| `$_.FullName -match "\\\d{6}\("` | `FOLDER_NAME_REGEX=r"^\d{6}\("` |
| `$_.BaseName + "_Latest.prproj"` | `BACKUP_SUFFIX="_Latest.prproj"` |
| `Copy-Item -Force` | 原子的コピー（`.part` → rename） |
| `New-ScheduledTaskTrigger -Daily -At 9:00AM` | `Scheduler`（`config.scheduleTime`、既定 09:00） |
| `-StartWhenAvailable` | 起動時の取りこぼし補填（常時有効） |
| `Unregister-ScheduledTask` | アプリアンインストールで完結（タスク登録を使わないため） |

## 16. 確定事項

- **全体監査（2026-09-30）**: 更新中の元ファイル・内容が変わったバックアップ・復旧不能な設定を安全側に扱う。Scheduler の実期限共有／壁時計再確認、終了時の非同期待機、UI の購読解除・古い応答破棄を採用。詳細と残る実環境検証は `AUDIT.md`。


- **設定保存の整合性（2026-09-30）**: 設定保存を直列化し、OS の自動起動状態を確認・変更してから設定を永続化する。自動起動変更／設定保存の失敗時は変更前の OS 状態への復元を試み、失敗内容を通知する。保存成功後は必ず Scheduler に再読込を要求し、送信失敗は「保存済み・再起動が必要」と明示する。実行状態3項目は UI の古い値で上書きしない。
- **削除予定の確認（2026-09-30）**: ダッシュボードから保存済み設定で読み取り専用のプレビューを実行する。実削除と同じ期限・除外・管理記録の判定で削除予定／保護対象／確認エラーを表示し、ファイル・管理記録・設定・ログは変更しない。Drive 待機は行わず、出力先不在は即時エラー。プレビューは確認時点の情報で、実行時に再判定する。自動実行を停止する確認ゲートや未管理ファイルの削除機能は追加しない。

- **失敗状態の保持（2026-09-30）**: `lastError` を実行状態として永続化し、次のジョブ完了まで失敗日時・理由を表示する。未設定・待機失敗・ジョブ中断では `lastRunAt` を更新せず、起動時の取りこぼし補填を維持する。

- **保持期限（2026-09-30）**: 元ファイルの最終更新から暦の2ヶ月でコピー対象外にし、管理記録で確認できる対応バックアップのみ削除する。元ファイルは残し、再編集時はバックアップを再開する。未管理ファイルは削除しない。

- **対象は `.prproj` のみ**。他拡張子は対象外。
- **同名衝突は考慮不要**。ユーザー側の運用でプロジェクト名が一意になる前提。
- **起動方式は常駐のみ**。CLI サブコマンドは持たず、UI 経由の操作に統一する。
- **スリープ時の取りこぼし**: 定時に PC がオフ／スリープだった場合、次回起動時に即時実行する（常時有効）。
- **Drive フォルダパス選択**: 自動検出した同期ルートをフォルダ選択ダイアログの起点にする方式（`DrivePathDetector`）。検出失敗時は通常のフォルダ選択ダイアログにフォールバック。
- **挙動を規定するパラメータはソース固定**: 対象拡張子・`Auto-Save` 除外・`^\d{6}\(` 正規表現・Drive 待機 300 秒・`_Latest.prproj` サフィックスは 8.3 の定数として持ち、`config.json` には含めない。変更したい場合はソース改修。
- **ユーザー設定の追加除外（2026-04-24 追加）**: 上記の固定条件は維持しつつ、`excludedFolders`（絶対パス）と `excludedFolderNames`（名前パターン、大文字小文字無視）を `config.json` に追加。現場運用で都度変わる除外（`Proxy` / `Cache` / 特定プロジェクト配下の一部など）を UI から編集できるようにする。どちらも空配列でも可。
- **設定ファイルの保存先**: インストーラー版は更新時にインストール先が作り直されても消えないよう `%USERPROFILE%\yuru-auto-backup-gdrive\` に置く。手動配置のポータブル版のみ、実行ファイルと同じディレクトリ配下の `data/` に `config.json` と `logs/backup.log` を保持する。
