# エンジン移植の状況（カバレッジ）

SearXNGのエンジン定義を移植しています。Google関連の一部エンジンと連携は意図的に削除しています。**型を移植したことと、既定で有効なこと・アクセスの許可があること・安定して結果を返すことは別です。** 数値は `tools/coverage.py` で再計測できます。現在の設定上の有効件数・監査状況は[プロバイダー台帳](PROVIDERS.md)を確認してください。

```bash
python3 tools/coverage.py /path/to/searxng settings.toml
```

## 過去の移植時点の記録（現在の既定設定ではない）

以下は過去の移植時点の記録で、現在の対応数や動作保証ではありません。

| 指標 | 値 |
| --- | ---: |
| 上流のエンジン型 | 226 |
| 本移植のエンジン型 | **226 / 226（100%）** |
| 上流エントリのうち型を実装済み | **352 / 352（100%）** |
| 上流にない追加エンジン | yahoo_japan（日本語検索） |
| 本移植の設定エントリ | 275 |
| 当時の既定で有効なエントリ | 約170（**歴史的な値**。現在の既定は273エントリ中34件） |
| テスト | ユニット 722 + 統合 14 |

## 移植を支える仕組み

1. **宣言的エンジン**（`engine: xpath` / `json_engine` / `mediawiki`）で上流 `settings.yml` の定義をそのまま取り込めます。
2. **コードエンジン**は 1 エンジン 1 ファイル（`src/engines/<name>.rs`）。すべてに外部ネットワーク不要のフィクスチャテストを付けています。
3. **外部 bang**: 上流の圧縮トライを平坦化し、Google 関連を除外。
4. **登録の自動化**: `tools/register_engines.py` が `// engine-id:` 行を読み、`mod.rs` の宣言・ディスパッチと `settings.toml` のエントリを同期します。
5. **壊れたエンジンは起動時にスキップ**: 上流と同じく、初期化に失敗したエンジンは警告を出して除外し、残りで検索を続けます。

## エンジン型の内訳

| 分類 | 数 | 状態 |
| --- | ---: | --- |
| スクレイパー / HTTP API | 約200（移植時） | 型の実装と、利用条件に沿った実行の可否は別。多数は現在既定無効 |
| 検索バックエンド（HTTP） | 3 | `elasticsearch` / `sorl` / `meilisearch`（`base_url` + index / collection を設定） |
| データベース・ローカル検索・関連検索バックエンド | 8（移植時の分類） | PostgreSQL、MySQL、MariaDB、MongoDB、SQLite、Valkeyは検索処理を実装。RecollとOpen Semantic SearchはHTTP検索先に接続する |
| MediaWiki / 宣言型 | 40 以上 | `tools/import_engines.py` で上流定義から生成 |

### DB ドライバ直結エンジンについて

`postgresql`、`mysql_server`、`mariadb_server`、`mongodb`、`sqlite`は対応するデータベースへの検索処理を持ち、`valkey_server`はRESPで検索します。`recoll`と`opensemantic`はHTTP経由で既存の検索サービスを使うアダプターです。認証情報、実際のデータベース、設定に依存するため、型が存在するだけで接続成功や上流との完全互換を保証するものではありません。対応状況は各エンジンの実装とテストで検証してください。

### 実装済みでも既定で無効なもの

上流と同じく、次は認証情報や自前インスタンスの設定が必要なため `disabled = true` にしています。

- API キー: `braveapi`, `kagi`, `exaapi`, `deepseek`, `wolframalpha_api`, `springer`, `flickr`, `giphy`, `soundcloud`, `marginalia`, `jina`, `ollama`, `spotify` など
- 自前インスタンス: `lemmy`, `mastodon`, `peertube`, `sepiasearch`, `discourse`, `torznab`, `annas_archive`, `yacy`, `zlibrary`, `searx 系`
- 翻訳系: `deepl`, `libretranslate`, `translated`

## 移植時の意図的な差分

- **2 リクエスト必要なエンジン**（PubMed の esearch→efetch、Open-Meteo のジオコーディング、Ahmia のトークン取得など）は、単一リクエストで取得できる代替エンドポイントを使うか、トークン等を設定値から受け取ります。各ファイル冒頭に明記しています。
- **JavaScript 実行が必要なページ**は対象外、または JSON エンドポイントに切り替えています。
- **アシスタント型エンジン**（`calculator` など）はプラグイン / アンサーラーとして実装しています。
- **天気・通貨・翻訳**は answerer / engine のどちらでも扱えるよう、アンサーラーと engine の両方を用意しています。

## サブシステム（エンジン以外）

| 上流 | 状態 |
| --- | --- |
| プラグイン 12 種 | 11 移植済み（hostnames / tracker_url_remover / time_zone / calculator / hash / oa_doi / self_info / unit_converter / ahmia_filter / tor_check / infinite_scroll） |
| アンサーラー | statistics / random / weather |
| favicons | 実装（`/favicon_proxy`、キャッシュ付き） |
| i18n | 57 ロケール（`tools/import_translations.py` で生成） |
| wikidata インフォボックス | `src/wikidata_infobox.rs` |
| valkey / redis キャッシュ | メモリ内TTL/LRUとValkey/Redisバックエンドを実装済み。複数プロセス間の送信レート制御まで共有するものではない |
| botdetection / limiter.toml | 部分互換。詳細は [COMPATIBILITY.md](COMPATIBILITY.md) と現在の実装を参照 |
