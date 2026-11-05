# Standard 変更前構成のベースライン実測（2026-11-05）

## 目的

計画書PR 4（回帰測定）の公平な比較のため、最新ベースで「変更前構成」（CodexのPR 1で既定無効化した11系統を有効に戻した状態）を実測し、Codexが同一条件で測定した「変更後」と比較した。

## 方法

- ベース: merge b986b79（v0.28.0系 origin/main 取り込み済み）+ 作業ツリーのバイナリ（RustXNG 0.28.0、PR 1〜3込み）
- 変更前構成: settings.toml のコピーで11系統の disabled = true を disabled = false に明示した --config オーバレイ（/tmp/rustxng-baseline-settings.toml、有効エンジン119）。変更後は108
  - 補足: 設定マージは「キーが無い場合は既定値が残る」ため、無効化行の削除では戻らず disabled = false の明示が必要だった
- サーバ: 変更前 127.0.0.1:29836 / 変更後 127.0.0.1:29835（Codexが実施）
- 質問セット: bench/quality-questions-100.json、--delay 1.1、同一バイナリ・同一ローカル索引（index-docs / index-wikipedia）
- 測定時刻: 変更後 20:42–20:46、変更前 21:00–21:03（外部要因の差は残る点に注意）

## 結果

| 指標 | 変更前（11系統有効） | 変更後（11系統無効・旧バイナリ） | 変更後（新バイナリ・索引修正込み） | 変更前との差 |
| --- | ---: | ---: | ---: | ---: |
| hit@1 | 70 | 55 | 55 | -15 |
| hit@5 | 84 | 68 | 69 | -15 |
| MRR@10 | 0.762 | 0.608 | 0.610 | -0.152 |
| 中央値レイテンシ | 981 ms | 807 ms | 826 ms | 約 -160 ms |
| エラー | 0 | 0 | 0 | 0 |

変更後の新バイナリ実測（Codex、21:09再ビルド・ローカル索引の製品名一致修正込み）は、旧バイナリの変更後とほぼ同じ値だった。索引修正の効果はこの質問セットでは hit@5 +1 の範囲にとどまる。

## 失われた14問の帰属

変更後に上位10件から外れた14問（新規に取れた問いは0）は、すべて変更前では bing か seznam が回答していた（seznam 9問、bing 7問、一部重複）。bing news・bing_images・bing_videos・imdb・unsplash・flickr・ebay・pinterest・rottentomatoes の9系統は、この質問セットでは上位回答に寄与していなかった。

## 含意

- 品質低下（hit@1 -15）は Bing（本体）と Seznam の一般検索に集中している
- 変更後は中央値レイテンシが約160〜174 ms改善した。応答するエンジンが減ったためと考えられる
- PR 4の「利用条件を確認した正式APIやローカル索引で補う」対応は、一般検索の代替（Bing/Seznam相当のカバー）に絞るのが有効とみられる
- 保存済みデータ（11月4日・旧ベース）の 72/87/660ms と今回の変更前 70/84/981ms はhit数が近いが、レイテンシの差にはベースと計測条件の違いが含まれるため、直接の比較には使わない

## ファイル

- 変更前: docs/audit/2026-11-05/rustxng-standard-quality-before.json
- 変更後（旧バイナリ）: docs/audit/2026-11-05/rustxng-standard-quality-after.json（Codex測定）
- 変更後（新バイナリ）: /tmp/rustxng-standard-quality-final.json（Codex測定）
- SealQA 254問のbefore/after: docs/audit/2026-11-05/sealqa-baseline-2026-11-05.md
