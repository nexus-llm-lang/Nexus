LLM（大規模言語モデル）によるコード生成を前提とした、Webバックエンド向けプログラミング言語 **「Nexus（ネクサス）」** の設計仕様書です。

---

# Nexus Programming Language Specification (v1.0)

## 1. Design Philosophy (設計思想)

**"Constraint is Context" (制約こそが文脈である)**

Nexusは人間が書くための言語ではなく、**「人間が仕様（型・インターフェース）を定義し、LLMが実装を埋める」**ための言語です。

1. **LLM-Native Syntax:**
* トークン予測の不確実性を排除するため、**「閉じる場所（End Token）」**や**「変数の役割（Prefix）」**を言語仕様レベルで強制する。
* 暗黙的なルール（省略記法）を廃止し、全ての操作を明示的（Explicit）にする。


2. **Anti-Hallucination:**
* 存在しない関数や変数の幻覚を防ぐため、徹底した**Strict ANF（正規形）**と**Labeled Arguments（ラベル引数）**を採用。


3. **Verification Oriented:**
* **線形型（Linear Types）とスタック限定参照**により、ライフタイム注釈なしでメモリ安全性とリソース管理（Close忘れ防止）をコンパイル時に保証する。



---

## 2. Syntax & Structure (構文と構造)

LLMのAttention機構が「迷子」にならないよう、ブロック構造と処理の流れを厳格化します。

### 2.1 Block Terminators (ブロック終端)

汎用的な `}` や `end` を廃止し、ブロックの種類に応じた終端トークンを強制します。

* **Function:** `fn ... do ... endfn`
* **Match:** `match ... do ... endmatch`
* **If/Else:** `if ... then ... endif`
* **Concurrency:** `conc do ... endconc`
* **Guard:** `guard ... else ... endguard`

### 2.2 Strict ANF & Labeled Arguments (厳格な正規形とラベル)

* **No Nesting:** 関数呼び出しの引数に、別の関数呼び出しを書くことを禁止します。必ず変数に束縛します。
* **Full Labels:** 全ての引数はラベル付き（Keyword Arguments）でなければなりません。
* **No Pipes:** パイプライン演算子 `|>` は廃止（ラベル引数との競合回避のため）。

```nexus
// ❌ Bad (Nested & Positional)
save(json(data), true)

// ⭕ Good (Nexus Style)
let json_str = json_encode(val: data)
save_file(content: json_str, overwrite: true)

```

### 2.3 Raw Strings

エスケープシーケンスによるLLMの混乱を防ぐため、LuaスタイルのRaw Stringを採用します。

```nexus
let sql = [[ SELECT * FROM users WHERE id = ? ]]

```

---

## 3. Type System & Memory Safety (型とメモリ)

Rustのようなライフタイム注釈 `&'a` を排除し、**変数の接頭辞（Sigil）**によって物理法則（可変性・移動）を決定します。

### 3.1 Immutable (Default)

接頭辞なしの変数は不変です。

```nexus
let x = 10

```

### 3.2 Stack-Local Mutability (`~` Tilde Namespace)

`~` で始まる変数は可変ですが、**現在のスコープ（スタック）から脱出できません**。

* **Gravity Rule:** 戻り値にできない。クロージャや並行タスクにキャプチャさせない。
* **Purpose:** LLMの学習データにおいて `~` は出現頻度が低いため、本当に必要な箇所（ループカウンタ等）以外での使用を抑制します。

```nexus
let ~counter = ref(0)
~counter <- ~counter + 1

```

### 3.3 Linear Resources (`%` Percent Namespace)

`%` で始まる変数は線形型（Linear Type）です。**必ず一度だけ消費（Consume）または移動（Move）**されなければなりません。

* **Baton Rule:** 関数から関数へとバトンリレーのように渡される。
* **Safety:** リソースの閉め忘れ（Close忘れ）や二重解放をコンパイルエラーにします。

```nexus
let %tx = db.begin()
// ...
db.commit(tx: %tx) // Must consume

```

---

## 4. Effects & Architecture (副作用と構成)

### 4.1 Native Effects

関数シグネチャに副作用をタグ付けします。

* `<IO>`: 入出力
* `<Net>`: ネットワーク
* `<Exn>`: 例外送出
* `<FFI>`: 外部関数呼び出し

### 4.2 Ports & Handlers (Dependency Injection)

インターフェース（Port）と実装（Handler）を分離し、LLMにテストコード（Mock）を容易に書かせます。

```nexus
port Database do
  fn save(data: str) -> unit <IO>
endport

```

### 4.3 Modules

* **Explicit Import Only:** `import *` は禁止。使用する関数を全て列挙させる。
* **Type-First:** ファイルの先頭に型定義を置くことを推奨（LLMのコンテキスト固定のため）。

---

## 5. Comprehensive Example (コード例)

ユーザー登録処理の例です。
リソース管理（`%tx`）、並行処理（`conc`）、可変参照（`~`）、アーリーリターン（`guard`）を含みます。

```nexus
// --- Imports ---
import { db_driver } from "std/db"
import { log } from "std/log"
import { json } from "std/json"

// --- Type Definitions ---
type User = {
  id: i64,
  name: str,
  email: str
}

// --- Port Definition ---
port UserRepository do
  fn exists(tx: %Tx, email: str) -> Result<bool, str> <IO>
  fn create(tx: %Tx, u: User) -> Result<%Tx, str> <IO>
endport

// --- Main Logic ---
pub fn register_user(name: str, email: str) -> Result<unit, str> <IO, Net> do

  // 1. Linear Resource (Transaction Start)
  let %tx = perform db_driver.begin_tx()

  // 2. Mutable Reference (Local Counter)
  let ~retry_count = ref(0)

  // 3. Structured Concurrency
  // 並行タスクが完了するまでこのブロックを抜けない
  conc do
    task "audit_log" do
       perform log.info(msg: "Registration started: " + email)
    endtask
  endconc

  // 4. Logic with Strict ANF & Early Return
  let exists_res = perform UserRepository.exists(tx: %tx, email: email)

  // Guard Clause for Happy Path
  // matchの結果を変数で受け、エラーなら即リターン
  match exists_res do
    case Err(msg) ->
      perform db_driver.rollback(tx: %tx)
      return Err(msg)
    
    case Ok(is_exists) ->
      if is_exists then
        perform db_driver.rollback(tx: %tx)
        return Err("User already exists")
      endif
  endmatch

  // データ構築 (JSON-like Record)
  let new_user = {
    id: 0, // Auto-increment
    name: name,
    email: email
  }

  // 5. Linear Resource Threading
  // %tx を渡して、新しい %tx を受け取る (Shadowing)
  let create_res = perform UserRepository.create(tx: %tx, u: new_user)

  match create_res do
    case Err(msg) ->
      perform db_driver.rollback(tx: %tx)
      return Err("Create failed: " + msg)

    case Ok(%new_tx) ->
      // 6. Final Consumption (Commit)
      perform db_driver.commit(tx: %new_tx)
      return Ok(())
  endmatch
endfn
```
