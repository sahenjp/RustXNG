# RustXNG

[English](README.md) · [한국어](README.ko.md)

RustXNGは、Rustで作られたセルフホスト型メタ検索エンジンです。[SearXNG](https://github.com/searxng/searxng)をもとに作りました。複数の検索サービスから結果を集めて、JSON APIやMCPを使ってAIエージェントに渡せます。

Web全体を独自に索引化する検索エンジンではありません。必要なサイトをクロールしてローカル索引を作ったり、Wikipediaの公式ダンプから索引を作ったりできます。

## 使い始める

[Releases](https://github.com/sahenjp/RustXNG/releases)からOSに合うバイナリを入手できます。実行前に、公開されているSHA-256チェックサムを確認してください。Cargoからも導入できます。

<!-- x-release-please-start-version -->
```bash
cargo install --git https://github.com/sahenjp/RustXNG.git --tag v0.31.0 --locked
```
<!-- x-release-please-end -->

ソースから導入するにはRust 1.88以降とCのビルド環境が必要です。詳しくは[エージェント向け導入ガイド](docs/AGENT-SETUP.md)を見てください。

サーバーを起動します。

```bash
rustxng --bind 127.0.0.1 --port 29483
```

別のターミナルから検索できます。

```bash
curl --fail --get 'http://127.0.0.1:29483/search' \
  --data-urlencode 'q=tokio runtime documentation' \
  --data 'format=json' --data 'max_results=5'
```

結果にはタイトル、URL、抜粋、検索元などが含まれます。検索元の指定を厳密に確認したい場合は`strict_engines=1`を使えます。言語や検索元、ドメイン、期間、本文取得なども指定できます。詳しくは[APIの互換性と拡張](docs/COMPATIBILITY.md)を見てください。

ブラウザーからは<http://127.0.0.1:29483/>を開いてください。検索結果はJSONのほか、CSV、RSS、HTMLでも取得できます。

## AIエージェントから使う

エージェント自身が導入から動作確認まで行う手順は、[エージェント向け導入ガイド](docs/AGENT-SETUP.md)にまとめています。配布バイナリの選択とチェックサム検証、ループバックでの起動、小さな検索での動作確認、報告の書式までです。

組み込みのMCPサーバーを起動できます。

```bash
rustxng mcp
```

MCPには、読み取り専用のツールが2つあります。

- `search`：検索結果を順位順に返します。デフォルトは5件、最大10件で、抜粋は1件500文字までです。
- `read_page`：同じセッションの`search`で見つけたページから本文を取得します。`query`を指定すると、関連する文章を最大3件選びます。

`read_page`はデフォルトで4,000文字、最大20,000文字の本文を返します。続きを読みたいときは`offset`と`next_offset`を使えます。MCPの出力量は文字数・バイト数で制限でき、`tokenizer`と`max_payload_tokens`を指定すれば`structuredContent`のトークン数も制限できます。詳しくは[出力の仕様](docs/AGENT-OUTPUT.md)を見てください。

本文の取得時には接続先とリダイレクトを確認し、プライベートIPへの接続を拒否します。robots.txtにも従います。取得したWebページは、AIへの指示ではなく外部の情報として扱ってください。

HTTP APIでも`include_raw_content=1`で本文を取得できます。PDFの文字抽出にはPopplerが必要です。スキャン画像のOCRには別のツールが必要です。このとき各結果に`raw_content_status`（`ok` / `not_requested` / `too_large` / `fetch_failed` / `robots_blocked` / `extract_failed` / `render_failed` / `unknown`）と、取得できた理由を示す短い`raw_content_error`が付きます。本文上限は`raw_content_max_bytes`で変更でき（既定4MiB、上限32MiB）、レンダラー失敗時は既定で静的HTMLの本文を保持します（`page_renderer_fallback = "static"`）。JavaScriptで表示されるページには、必要に応じて[Firefoxレンダラー](tools/page-renderer/README.md)を設定します。

## 検索先の構成

起動時に`--profile`で検索先を切り替えられます。デフォルトは`standard`です。

Standardに含まれる検索先は、API・RSS・JSON形式のエンドポイントだけで、人間向けの検索結果ページ（HTML）の取得は行いません。各検索元の確認済みの条件は[docs/PROVIDERS.md](docs/PROVIDERS.md)に記録しています。FullはHTMLを取得する検索元を追加するもので、規約で自動取得を禁じる検索元を含みます。Fullで起動すると、その確認を促す通知が開始時に表示されます。

| 構成 | コマンド | 検索先 |
| --- | --- | --- |
| Standard | `rustxng` | 利用条件を確認・記録した、キー不要の34エントリと自前の索引 |
| Clean | `rustxng --profile clean` | クロール索引とWikipediaダンプ索引だけを検索。外部検索APIは使わない |
| Full | `rustxng --profile full` | 追加のHTML検索元と、有料のExa Search（APIキーが必要） |

StandardとFullでは、利用する外部検索サービスに検索語を送信します。Cleanはローカル索引だけを検索しますが、本文取得などを明示的に使うと外部通信が発生する場合があります。MCPでも同じプロファイルを選べます。

Fullには、利用規約で自動取得が制限されていたり、許可が確認できていなかったりする検索元も含まれます。**個人利用でも規約は適用されます。必要な許可を確認できない検索元は有効にしないでください。** StandardやCleanも、あらゆる利用目的について法的な許可を保証するものではありません。[検索元の利用条件](docs/PROVIDERS.md)を確認してください。

## 導入と設定

Dockerでも起動できます。

```bash
docker run --rm -p 127.0.0.1:29483:29483 \
  -e SEARXNG_PORT=29483 ghcr.io/sahenjp/rustxng:latest \
  --bind 0.0.0.0 --port 29483
```

デフォルトではローカル接続のみを受け付けます。外部に公開するときは、認証やアクセス制限を設定してください。

設定はバイナリに含まれています。変えたい項目だけをTOMLファイルに書いて指定できます。例えば、次の`my-settings.toml`を使います。

```toml
[server]
idle_timeout = 600
```

```bash
rustxng --config my-settings.toml
```

更新の確認には`rustxng --check-update`、対応するバイナリの更新には`rustxng --update`を使います。パッケージ管理やDockerで導入した場合は、それぞれの方法で更新してください。

## 検索精度について

11月5日のStandard測定では、技術文書100問のHit@5が**67〜69/100**でした。検索元の見直し前は84/100で、検索の網羅性にはまだ改善の余地があります。[測定記録](docs/ISSUE-RESOLUTION-2026-11-05.md)

ローカル索引だけを使うCleanは、SealQA 254問の測定でHit@10が**85/254**、応答時間の中央値が**42.7ms**でした。[Cleanの測定記録](docs/CLEAN-PROFILE.md)

これらは異なる質問集で測った過去の結果で、v0.30.0の同条件比較ではありません。AIの回答正解率を示すものでもありません。

数値はすべて開発者自身の環境で測った自己申告の値です。機械や回線が変われば結果も変わります。信頼する前に[docs/BENCHMARK.md](docs/BENCHMARK.md)の手順で自分の環境で再測定してください。

ExaやTavilyとの比較には構造的な差もあります。RustXNGは複数の検索元の結果を1つの順位リストに統合するメタ検索で、両APIは単一の索引を返します。統合するだけで命中率は上がるため、測定差がそのまま検索元ごとの品質差を示すわけではありません。これらの数値は工程の記録として読み、優劣の結論にはしないでください。

## 関連ドキュメント

- [エージェントへの導入と動作確認](docs/AGENT-SETUP.md)
- [検索元の利用条件と設定](docs/PROVIDERS.md)
- [自前クローラーと索引](docs/LOCAL-INDEX.md)・[Wikipediaの索引](docs/WIKIPEDIA-INDEX.md)
- [SearXNGとの互換性](docs/COMPATIBILITY.md)・[過去のベンチマーク](docs/BENCHMARK.md)
- [セキュリティ](SECURITY.md)・[開発への参加](CONTRIBUTING.md)・[リリース手順](docs/RELEASING.md)

## 開発への参加とライセンス

不具合の報告や提案は[Issues](https://github.com/sahenjp/RustXNG/issues)と[Discussions](https://github.com/sahenjp/RustXNG/discussions)へお願いします。PRも歓迎します。詳しくは[CONTRIBUTING.md](CONTRIBUTING.md)、[GOVERNANCE.md](GOVERNANCE.md)、[MAINTAINERS.md](MAINTAINERS.md)を見てください。

RustXNGのライセンスは[AGPL-3.0-or-later](LICENSE)です。SearXNGと外部の貢献者への帰属表示は[NOTICE](NOTICE)に記載しています。コードのライセンスは、外部検索サービスのデータ取得や再配布を無条件に許可するものではありません。
