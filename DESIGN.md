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
| OS | Windows 11 Home |
| フレームワーク | Tauri (v2) |
| フロントエンド | TypeScript + 任意の UI フレームワーク（候補: React / Svelte） |
| バックエンド | Rust |
| 前提ソフトウェア | Google Drive for desktop（同期クライアント） |

## 4. バックアップ仕様（既存スクリプト踏襲）

### 4.1 対象ファイル

- 拡張子: `.prproj` のみ
- 監視元フォルダ配下を**再帰的**に探索
- フォルダ名条件: 祖先パスのいずれかのフォルダ名が正規表現 `^\d{6}\(` にマッチする必要がある（例: `250304(3)_クイズ...`）
  - 既存スクリプトの `FullName -match "\\\d{6}\("` に相当

### 4.2 除外条件

- パスに `Auto-Save` を含むもの（Premiere Pro 自動保存）を常に除外

### 4.3 出力先・命名規則

- 出力先: **単一のフォルダ**（サブフォルダ構造は作らない、平置きコピー）
- ファイル名: `<元ファイル名の BaseName>_Latest.prproj`
  - 例: `250304(3)_クイズ.prproj` → `250304(3)_クイズ_Latest.prproj`
- 既存ファイルは**常に上書き**（履歴は残さない）

### 4.4 実行タイミング

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
- `tokio` タイマーで、ローカル TZ の指定時刻（既定 09:00）まで待機 → 発火 → 次回時刻を再計算、のループ。
- 発火時に `BackupJob` を spawn。実行中に再度時刻が来た場合は**多重起動せず**ログに記録。
- **取りこぼし補填**: アプリ起動時、`lastRunAt` が本日でなく、かつ本日の実行時刻を過ぎていれば即時実行。

### 8.2 DriveWaiter
- 出力先パスの存在を 10 秒間隔で確認、最大 5 分（300 秒）リトライ。
- タイムアウトしたら `Err(DriveNotReady)` を返し、BackupJob は中止してログに記録。
- （既存スクリプトの `Test-Path $DEST` ループと同等）

### 8.3 固定フィルタ定数（ハードコード）

```rust
const TARGET_EXTENSION: &str = "prproj";
const EXCLUDE_PATH_KEYWORD: &str = "Auto-Save";
const FOLDER_NAME_REGEX: &str = r"^\d{6}\(";
const DRIVE_WAIT_SECONDS: u64 = 300;
const BACKUP_SUFFIX: &str = "_Latest.prproj";
```

既存スクリプトの運用ルールはユーザーが設定で変える意味がないため、定数として固定する。将来変更したい場合はソース改修で対応。

### 8.4 DrivePathDetector

Google Drive for desktop の同期ルート（例: `G:\マイドライブ`、`G:\共有ドライブ`）を自動検出する。出力先フォルダ選択時に、検出されたルートを起点にフォルダ選択ダイアログを開くことで、非エンジニアでも迷わず目的のフォルダに辿り着ける。

**検出ロジック（複数ソースを試行し、最初に見つかったものを採用）:**

1. **レジストリ参照**: `HKCU\Software\Google\DriveFS\Share` 配下のキー（`BaseDir` / マウントポイント情報）を読む。
2. **設定ファイル参照**: `%LOCALAPPDATA%\Google\DriveFS\` 配下の `root_preference_sqlite.db` や設定ファイルから現在のマウント情報を取得。
3. **ドライブレター総当たり**: `A:` から `Z:` まで走査し、ボリュームラベルに `Google Drive` を含むドライブを検出。
4. **慣習的パスのフォールバック**: `%USERPROFILE%\Google Drive`, `G:\マイドライブ`, `G:\My Drive` などを順に確認。

検出結果は候補リストとして返し、UI では 1 つ目を既定ハイライトする。検出ゼロ件の場合はユーザーに「Google Drive for desktop が起動しているか確認してください」と案内し、通常のフォルダ選択ダイアログにフォールバックする。

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
- `copy_atomic`: `dest.part` に書き込み → `fs::rename(dest.part, dest)` で上書き。
- **シリアル実行**（`.prproj` は小さく本数も少ないため並列化不要）。
- 完了時に `lastRunAt`, `JobSummary`（`copied` / `errors` の 2 指標）を ConfigStore / Logger に反映。

### 8.6 ConfigStore

**保存先の決定ロジック** (`AppDir::resolve()`):

1. 実行ファイルのあるディレクトリ配下の `data/` が**書き込み可能**ならそこを使用（ポータブル運用）。
2. 書き込み不可（例: `Program Files` 配下にインストール）の場合、`%USERPROFILE%\yuru-auto-backup-gdrive\` を作成して使用（フォールバック）。

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
  "source": "F:/pedantic制作",
  "destination": "H:/共有ドライブ/.../バックアップ",
  "scheduleTime": "09:00",
  "autoStart": true,
  "lastRunAt": "2026-04-23T09:00:12+09:00",
  "lastSummary": { "copied": 3, "errors": 0 }
}
```

6 項目のみ。ユーザーが意味のある選択肢を持つものだけを設定として持ち、挙動を規定するパラメータは 8.3 の定数で固定する。

### 8.7 Logger
- `<AppDir>/logs/backup.log` へ追記。1 日 1 ジョブなのでローテーションは行わず単一ファイル（必要に応じて手動削除）。
- 画面には直近 N 件をリングバッファで保持。
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
3. **ログ**: 時系列表示、フィルタ（成功／エラー）、1 ジョブ分を折りたたみ可。

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
- 「今すぐ実行」はスケジュールとは独立して BackupJob を spawn。次回予定は変えない。

### 10.4 終了時
- 進行中のジョブが完了するまで最大 30 秒待機 → プロセス終了。

## 11. Tauri Command インターフェース

| Command | 入出力 | 用途 |
| --- | --- | --- |
| `get_config` | `() -> Config` | 設定取得 |
| `update_config` | `(Config) -> Result<()>` | 設定更新（Scheduler に即時反映） |
| `pick_folder` | `(start_dir?: PathBuf) -> Option<PathBuf>` | フォルダ選択ダイアログ（起点ディレクトリを任意で指定） |
| `detect_drive_roots` | `() -> Vec<DriveCandidate>` | Google Drive 同期ルート候補を返す |
| `get_status` | `() -> Status` | 稼働状態 / 最終実行時刻 / 次回実行時刻 / 直近サマリ |
| `run_now` | `() -> Result<()>` | 手動バックアップ実行 |
| `list_recent_logs` | `(limit) -> Vec<LogEntry>` | 直近ログ取得 |
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
├─ memo.md
├─ package.json
└─ DESIGN.md
```

**インストール後のランタイム側**（ユーザー環境）:
```
<インストール先>/
├─ yuru-auto-backup-gdrive.exe
└─ data/                  # ← ここが書き込み可能なら配置
   ├─ config.json
   └─ logs/backup.log

（書き込み不可なら以下にフォールバック）
%USERPROFILE%\yuru-auto-backup-gdrive\
├─ config.json
└─ logs/backup.log
```

## 13. 主要依存クレート / ライブラリ

- Rust: `tauri`, `serde` + `serde_json`, `tokio`, `walkdir`, `regex`, `chrono`, `tracing`, `tauri-plugin-autostart`, `tauri-plugin-dialog`, `tauri-plugin-opener`
- フロント: `@tauri-apps/api`, UI フレームワーク（未定）

## 14. 実装ステップ

1. Tauri プロジェクト雛形を作成。
2. `AppDir::resolve()` を実装（実行ファイル隣の `data/` → `%USERPROFILE%` フォールバック）。
3. ConfigStore を実装（JSON 読み書き）＋ 設定 UI（監視元 / 出力先 / 実行時刻 / 自動起動）。
4. BackupJob を実装（`walkdir` + 固定フィルタ + `_Latest.prproj` 命名 + 原子的コピー）。
5. DriveWaiter を実装（最大 5 分ポーリング）。
6. DrivePathDetector を実装（レジストリ / 設定ファイル / ドライブレター走査 / 慣習パス、の優先順で検出）。
7. Scheduler を実装（`tokio` タイマー + 取りこぼし補填）。
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
| `$_.FullName -notmatch "Auto-Save"` | `EXCLUDE_PATH_KEYWORD="Auto-Save"` |
| `$_.FullName -match "\\\d{6}\("` | `FOLDER_NAME_REGEX=r"^\d{6}\("` |
| `$_.BaseName + "_Latest.prproj"` | `BACKUP_SUFFIX="_Latest.prproj"` |
| `Copy-Item -Force` | 原子的コピー（`.part` → rename） |
| `New-ScheduledTaskTrigger -Daily -At 9:00AM` | `Scheduler`（`config.scheduleTime`、既定 09:00） |
| `-StartWhenAvailable` | 起動時の取りこぼし補填（常時有効） |
| `Unregister-ScheduledTask` | アプリアンインストールで完結（タスク登録を使わないため） |

## 16. 確定事項

- **対象は `.prproj` のみ**。他拡張子は対象外。
- **同名衝突は考慮不要**。ユーザー側の運用でプロジェクト名が一意になる前提。
- **起動方式は常駐のみ**。CLI サブコマンドは持たず、UI 経由の操作に統一する。
- **スリープ時の取りこぼし**: 定時に PC がオフ／スリープだった場合、次回起動時に即時実行する（常時有効）。
- **Drive フォルダパス選択**: 自動検出した同期ルートをフォルダ選択ダイアログの起点にする方式（`DrivePathDetector`）。検出失敗時は通常のフォルダ選択ダイアログにフォールバック。
- **挙動を規定するパラメータはソース固定**: 対象拡張子・`Auto-Save` 除外・`^\d{6}\(` 正規表現・Drive 待機 300 秒・`_Latest.prproj` サフィックスは 8.3 の定数として持ち、`config.json` には含めない。変更したい場合はソース改修。
- **設定ファイルの保存先**: 実行ファイルと同じディレクトリ配下の `data/` に置く（ポータブル運用）。書き込み不可な場所にインストールされている場合は `%USERPROFILE%\yuru-auto-backup-gdrive\` にフォールバック。どちらも `data` 扱いとして `config.json` と `logs/backup.log` を保持。
