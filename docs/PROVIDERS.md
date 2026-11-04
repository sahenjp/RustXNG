# 検索プロバイダーの接続方式と利用条件

RustXNGは外部プロバイダーの検索結果を統合します。RustXNGのAGPLライセンスはコードの利用条件であり、
外部サイトの自動取得、検索結果の保存・再利用・再配布を許可するものではありません。
APIキーの要否、取得方式、認証、クォータ、利用目的、再配布条件はプロバイダーごとに確認してください。
公開APIやRSSであることだけでも、無制限の自動利用を意味しません。

## 2026-11-03の既定設定の変更

| エンジン | 実装の取得方式 | 一次資料で確認した制約 | 判断 |
| --- | --- | --- | --- |
| Yahoo! JAPAN | `search.yahoo.co.jp/search` のHTML解析 | [公式ヘルプ](https://support.yahoo-net.jp/PccSearch/s/article/H000015794)は、インデックス作成目的のクロール等の例外を除き、スクレイピング等の機械的な情報収集・利用を禁止 | 許諾未確認のため既定無効 |
| Mojeek | `www.mojeek.com/search` のHTML解析 | [Terms §3(5)](https://www.mojeek.com/about/terms.html)は認可API利用者を除く自動アクセスと、事前同意のないスクレイピングを禁止 | 許諾未確認のため既定無効 |
| Bing | `www.bing.com/search` のHTML解析 | [Microsoftサービス規約「BingおよびMSN」](https://www.microsoft.com/ja-jp/servicesagreement)には利用・再配布・独自製品での利用の制約がある | 適用される許諾と用途の確認を継続 |

これは個別利用の違法性の判断ではありません。許諾を確認できていないHTML経路を標準利用から外す保守上の判断です。
Yahoo! JAPANとMojeekのアダプターは残します。利用者が明示的に有効化する場合は、先に必要な許可・契約を確認してください。
既存のユーザー設定が `disabled = false` を明示していれば、その指定は引き続き優先されます。
既存の有効化設定も見直してください。プロキシ・UA変更やCAPTCHA回避を、利用条件の問題の解決策として扱いません。

## DeepSeek Search / Fetch API

`deepseek` エンジンは DeepSeek の Search API を検索先として使います。APIキーが必要で、既定では無効です。Search と Fetch は成功したリクエストごとに課金され、それぞれ QPS の上限があります。料金・上限・データの扱いは [DeepSeek の検索ガイド](https://api-docs.deepseek.com/guides/search)で確認してください。

本文取得を DeepSeek Fetch API に切り替える場合は、`settings.toml` の `[fetch]` で `provider = "deepseek"` を指定します。この場合、`read_page` と `include_raw_content` が取得対象の URL を DeepSeek へ送信します。既定の `direct` は、宛先をローカルで検証する従来の取得のままです。

## 全設定エントリの台帳

[機械可読CSV](providers.csv)は同梱設定の全271エントリを列挙します。[確認済み情報](provider-policy.json)から、
接続方式・規約URL・確認日・判断を載せます。それ以外は `unreviewed` とし、許可済みとはみなしません。
この台帳を作成したことは、全プロバイダーの利用条件の確認完了を意味しません。

`enabled_in_config` は `Settings::enabled_engines` と同じ設定上の条件です。初期化成功・認証の充足・
そのリクエストのカテゴリ選択・実際の応答を保証しません。設定エントリ数と登録済みエンジン数も区別してください。
`source_url_literals_unverified` はテスト部分を除くソース内URL文字列の候補です。説明用URLやテンプレートも含み、
実際の接続先・取得方式として確認済みの値ではありません。未確認の接続先は実装を読んで台帳へ追記します。

```bash
python3 tools/provider_inventory.py
python3 tools/provider_inventory.py --check
```

今後は有効な一般検索から順に、リクエスト先、HTML/API/RSS等の取得方式、認証、レート制限、保存・再配布条件、
規約の確認日、利用者の契約範囲を確認します。必要な許可が確認できるAPI等への置換も個別に検討します。

## 評価と移行

旧SealQAでは統合後の1位結果の代表エンジンが189/254問でYahoo! JAPANでした。この件数は、
Yahoo! JAPANを外した場合の性能低下量を表すものではありません。
既定構成を変更した後の性能は未測定であり、旧スコアを新構成の性能として扱いません。

新しい測定は、利用条件を確認した構成・設問・キャッシュ条件を固定して行います。
元の観測は履歴として保存し、[URL照合を修正した再集計](BENCHMARK-RESCORE-2026-11-03.md)も別に公開します。
