---
layout: default
title: Type System — Formal Rules
---

{% raw %}
# Type System — Formal Rules

This document defines the typing rules of Nexus as inference rules. It serves as a specification for property-based testing and as a reference for future mechanization.

## 1. Syntax

The abstract syntax of the core calculus. See [Syntax](../syntax) for the full surface syntax.

### Terms

$$\begin{array}{rcll}
\mu & ::= & \varepsilon \mid \% \mid \mathord{\sim} \mid @ \mid \& & \text{modality (plain / linear / ref / lazy / borrow)} \\[6pt]
e & ::= & \mu\,x \mid c & \text{variable with modality / constructor} \\
  & \mid & n \mid f \mid ch \mid b \mid s \mid () & \text{literals (int, float, char, bool, string, unit)} \\
  & \mid & e_1 \oplus e_2 & \text{binary operation} \\
  & \mid & f(\overline{\ell : e}) & \text{application} \\
  & \mid & \textbf{fn}~(\overline{\ell : \tau}) \to \tau;\,\rho;\,\rho~\textbf{do}~\overline{s}~\textbf{end} & \text{lambda} \\
  & \mid & \textbf{if}~e~\textbf{then}~\overline{s}~\textbf{else}~\overline{s} & \text{conditional} \\
  & \mid & \textbf{match}~e~\lbrace \overline{p \to s} \rbrace & \text{pattern match} \\
  & \mid & \lbrace \overline{\ell : e} \rbrace & \text{record} \\
  & \mid & e.\ell & \text{projection} \\
  & \mid & \textbf{raise}~e & \text{raise exception} \\
  & \mid & \textbf{handler}~x~[\textbf{require}~\rho]~\textbf{do}~\overline{\ell = e}~\textbf{end} & \text{handler (each } e \text{ is a lambda)} \\[6pt]
s & ::= & \textbf{let}~\mu\,x = e & \text{binding} \\
  & \mid & \textbf{let}~p = e & \text{destructuring} \\
  & \mid & \textbf{return}~e & \text{return} \\
  & \mid & \mathord{\sim}x \leftarrow e & \text{assignment} \\
  & \mid & \textbf{inject}~\overline{h}~\textbf{do}~\overline{s}~\textbf{end} & \text{capability injection} \\
  & \mid & \textbf{try}~\overline{s}~\textbf{catch}~\overline{p \to s}~\textbf{end} & \text{exception handling} \\
  & \mid & e & \text{expression statement} \\[6pt]
p & ::= & x & \text{variable pattern} \\
  & \mid & \_ \mid n & \text{wildcard / literal pattern} \\
  & \mid & c(\overline{\ell : p}) & \text{constructor pattern (} c \in \text{dom}(\Gamma) \text{)} \\
  & \mid & \lbrace \overline{\ell : p} \rbrace & \text{record pattern}
\end{array}$$

The core calculus omits several surface language features that are either desugared or handled as environment preconditions:

- **Constructors** ($c$) — assumed predefined in $\Gamma$ with a function type (n-ary) or a value type (nullary). Application $f(\overline{\ell : e})$ covers both function calls and constructor application. In patterns, $c$ is syntactically distinguished from variable patterns $x$.
- **Port declarations** — top-level declarations that populate $\Gamma$ with method signatures. Not terms; they are preconditions on $\Gamma$.
- **Exception / exception group declarations** — extend the $\texttt{Exn}$ sum type in $\Gamma$. Same status as port declarations.
- **While / for loops** — present in the surface syntax but desugared; not in the core calculus.
- **Import statements** — resolved before type checking; not modeled here.

### Types

$$\begin{array}{rcll}
\tau & ::= & b \mid \texttt{intlit} \mid \texttt{floatlit} & \text{base type / inference-internal numeric} \\
    & \mid & \alpha \mid {?}\alpha & \text{type variable / unification variable} \\
    & \mid & (\overline{\ell : \tau}) \to \tau;\, \rho;\, \rho & \text{function (params, return, capability, effect)} \\
    & \mid & \lbrace \overline{\ell : \tau} \rbrace & \text{record} \\
    & \mid & x\langle \overline{\tau} \rangle & \text{named type (e.g.\ } \texttt{Option}\langle\texttt{i64}\rangle\text{)} \\
    & \mid & [\tau] \mid [\lvert\,\tau\,\rvert] & \text{list / array (array is always linear)} \\
    & \mid & \%\tau \mid \mathord{\sim}\tau \mid \&\tau \mid @\tau & \text{linear / mutable ref / borrow / lazy} \\
    & \mid & \textbf{handler}\;x\;\rho & \text{handler for port } x \\[6pt]
b & ::= & \texttt{i32} \mid \texttt{i64} \mid \texttt{f32} \mid \texttt{f64} \mid {} & \\
  &     & \texttt{bool} \mid \texttt{char} \mid \texttt{string} \mid \texttt{unit} & \\[6pt]
\rho & ::= & \lbrace \overline{\tau} \rbrace \mid \lbrace \overline{\tau} \mid {?}r \rbrace & \text{row (closed / open with row variable } {?}r\text{)}
\end{array}$$

We write $\overline{X}$ for a finite sequence $X_1, \ldots, X_n$. $\alpha, \beta, \gamma$ range over type variables; ${?}\alpha$ denotes a unification variable introduced during inference (the distinction matters in generalization). $\lvert\overline{X}\rvert$ denotes the length of a sequence.

$\texttt{intlit}$ and $\texttt{floatlit}$ are inference-internal types assigned to integer and float literals before their concrete type is known. They are resolved early via unification or defaulted to $\texttt{i64}$/$\texttt{f64}$ at binding sites ([T-Let](#T-Let)).

### Modalities

The modality $\mu$ determines how a binding is introduced and used. $\varepsilon$ (plain) is elided in notation; we write $x$ for $\varepsilon\,x$. The modalities $\%$, $\mathord{\sim}$, $\&$, $@$ correspond to the surface sigils `%x`, `~x`, `&x`, `@x`. A binding with type $\%\tau$, $@\tau$, or $[\lvert\,\tau\,\rvert]$ has usage $q = 1$ (linear); all others have $q = \omega$. $@\tau$ denotes a suspended computation (one-shot thunk). $\mathord{\sim}\tau$ is a mutable reference cell. $\&\tau$ is a read-only borrow.

In expression position, $\mu\,x$ with $\mu \in \{\varepsilon, \%, \mathord{\sim}\}$ is a variable reference. $@e$ (force) and $\&x$ (borrow) subsume the $\mu = @$ and $\mu = \&$ cases and are listed as separate expression forms since force applies to any expression.

### Row Types

The row type $\rho$ is used for both the effect position ($\rho_e$, in $\textbf{throws}$) and the capability position ($\rho_q$, in $\textbf{require}$) of function types. Both positions share the same structure — row extension, unification, row variable instantiation — so no separate syntactic category is needed. The distinction is semantic: $\rho_e$ ranges over exception types, $\rho_q$ ranges over capabilities ($\texttt{PermFs}$, $\texttt{PermNet}$, etc.). No kind system enforces this; the invariant is maintained by the introduction rules (T-Raise adds to $\rho_e$; handler declarations add to $\rho_q$).

In the current language, the only effect is checked exceptions: $\rho_e$ contains at most $\texttt{Exn}$ (an extensible sum type to which user-declared `exception` adds variants). The rules are stated for a general row $\rho_e$, but in practice every $\rho_e$ is either $\lbrace\rbrace$ (pure) or $\lbrace \texttt{Exn} \rbrace$ / $\lbrace \texttt{Exn} \mid {?}r \rbrace$.

The capability names $\texttt{PermFs}$, $\texttt{PermNet}$, $\texttt{PermConsole}$, $\texttt{PermRandom}$, $\texttt{PermClock}$, $\texttt{PermProc}$, $\texttt{PermEnv}$ correspond to WASI interface grants at runtime. See [WASM and WASI](../../env/wasm) for the complete mapping.

---

## 2. Typing Rules

### Environment and Usage

$$S ::= \forall \overline{\alpha}.\,\tau \qquad q \in \lbrace 1, \omega \rbrace \qquad \Gamma ::= \lbrace\; \overline{x :^{q} S} \;\rbrace$$

$S$ is a type scheme. Each binding in $\Gamma$ carries a **usage** annotation $q$: $1$ (must be used exactly once) or $\omega$ (may be used any number of times). The sigils $\%$ and $@$ introduce bindings with $q = 1$; all others have $q = \omega$.

The split operation $\Gamma_1 \otimes \Gamma_2 = \Gamma$ distributes bindings to sub-derivations. For each $x :^{q} S \in \Gamma$, the split assigns $x :^{q_1} S \in \Gamma_1$ and $x :^{q_2} S \in \Gamma_2$ according to:

$$\begin{array}{c|ccc}
q_1 + q_2 = q & q_2 = \cdot & q_2 = 1 & q_2 = \omega \\
\hline
q_1 = \cdot & \cdot & 1 & \omega \\
q_1 = 1 & 1 & — & — \\
q_1 = \omega & \omega & — & \omega
\end{array}$$

$\cdot$ means the binding is absent from that side of the split. A linear binding ($q = 1$) splits as $(1, \cdot)$ or $(\cdot, 1)$ — the choice of which side receives it is arbitrary (determined by the derivation). An unrestricted binding ($q = \omega$) splits as $(\omega, \omega)$ — both sides share it. "$—$" is forbidden ($1 + 1$ would use a linear resource twice).

### Auxiliary Functions

$$\begin{array}{rcl}
\text{typeof}(v) & = & \begin{cases}
\texttt{intlit} & \text{if } v \text{ is an integer literal} \\
\texttt{floatlit} & \text{if } v \text{ is a float literal} \\
\texttt{bool} & \text{if } v \in \lbrace \texttt{true}, \texttt{false} \rbrace \\
\texttt{string} & \text{if } v \text{ is a string literal} \\
\texttt{char} & \text{if } v \text{ is a character literal} \\
\texttt{unit} & \text{if } v = ()
\end{cases} \\[4pt]
\text{occurs}(\alpha, \tau) & & \text{holds iff } {?}\alpha \in \text{fv}(\tau) \\[4pt]
\text{fields}(\tau) & = & \begin{cases}
\overline{\ell : \sigma} & \text{if } \tau = \lbrace \overline{\ell : \sigma} \rbrace \\
\overline{\ell : \sigma[\overline{\alpha := \tau'}]} & \text{if } \tau = x\langle\overline{\tau'}\rangle,\;\text{typedef}(x) = \forall\overline{\alpha}.\,\lbrace\overline{\ell : \sigma}\rbrace \\
\text{error} & \text{otherwise}
\end{cases} \\[4pt]
\text{variants}(\tau) & = & \text{constructors of } \tau \\[4pt]
\text{exhaustive}(\tau, \overline{p}) & & \text{holds iff } \text{check}(M, [\tau]) = \text{ok},\; M = [[p_1], \ldots, [p_n]]
\end{array}$$

$\text{typedef}(x)$ denotes the definition of named type $x$ in the global type-definition environment.

Other functions are introduced where first used: $\text{linear}$, $\text{autoDrop}$ (Linearity), $\text{strip}$ (Pattern Matching), $\text{open}$ and $\text{selectInt}$/$\text{selectFloat}$ (Expressions), $\text{default}$, $\text{wrapSigil}$ (Statements), $\text{merge}$, $\text{ports}$ (Statements), $\text{tail}$ (Expressions).

### Linearity

$\text{linear}(\tau)$ is a structural (recursive) predicate: holds if $\tau$ is $\%\sigma$, $@\sigma$, or $[\lvert\,\sigma\,\rvert]$ at the outermost level, or if any transitive component of $\tau$ is linear (fields of records, type arguments of named types, element types of lists, elements of rows). Example: $\text{linear}(\text{Pair}\langle\%\texttt{i64}, \texttt{i64}\rangle)$ holds.

$\text{autoDrop}(\tau)$ holds if the innermost non-modality type of $\tau$ (recursively stripping $\%$, $@$, $\&$, $\mathord{\sim}$) is in $\lbrace b, \texttt{intlit}, \texttt{floatlit}, [\lvert\,\sigma\,\rvert] \rbrace$. Types whose linear wrapper can be silently discarded.

Linearity is entirely structural: the split $\otimes$ ensures each linear binding ($q = 1$) goes to exactly one sub-derivation, and branching constructs give both arms the same portion of $\Gamma$.

Two additional behaviors are embedded in specific rules rather than stated as standalone inference rules:

- **Weakening** (in [T-App](#T-App)): when a parameter has type $\%\tau$ and the argument has type $\sigma$ with $\neg\text{linear}(\sigma)$, $\text{unify}(\sigma, \tau)$ is used instead of $\text{unify}(\sigma, \%\tau)$. This applies only to the linear modality $\%$, not to $@$ or other linear-producing forms.
- **Closure linearization** (in [T-Lambda](#T-Lambda)): when a lambda captures any linear binding from $\Gamma$, its closure type is wrapped with $\%$ (making the closure itself linear).

### Unification

Unification is symmetric: $\text{unify}(\tau_1, \tau_2) = \text{unify}(\tau_2, \tau_1)$ unless otherwise noted. The rules below are written with the "interesting" argument on the left; the symmetric case is implied. U-Borrow and U-Expand are intentionally asymmetric and do **not** have symmetric counterparts.

$$\dfrac{}{\text{unify}(\tau, \tau) = \emptyset} \;\textsc{U-Refl}$$

$$\dfrac{\neg\text{occurs}(\alpha, \tau)}{\text{unify}({?}\alpha, \tau) = \lbrace {?}\alpha := \tau \rbrace} \;\textsc{U-Var}
\qquad
\dfrac{\text{occurs}(\alpha, \tau)}{\text{unify}({?}\alpha, \tau) = \text{error}} \;\textsc{U-Occurs}$$

$$\dfrac{}{\text{unify}(\texttt{intlit}, \texttt{i32}) = \emptyset} \quad
\dfrac{}{\text{unify}(\texttt{intlit}, \texttt{i64}) = \emptyset} \;\textsc{U-IntLit}$$

$$\dfrac{}{\text{unify}(\texttt{floatlit}, \texttt{f32}) = \emptyset} \quad
\dfrac{}{\text{unify}(\texttt{floatlit}, \texttt{f64}) = \emptyset} \;\textsc{U-FloatLit}$$

Numeric literal types are not rewritten by unification — the substitution is $\emptyset$, merely checking compatibility. U-Var, U-IntLit, and U-FloatLit apply in both argument orders via the symmetry convention above.

$$\dfrac{
  \begin{array}{l}
  \lvert\overline{p_1}\rvert = \lvert\overline{p_2}\rvert \qquad
  \forall i.\;\ell_i^1 = \ell_{\pi(i)}^2 \qquad
  \text{unify}(\tau_i^1, \tau_{\pi(i)}^2) \\[2pt]
  \text{unify}(\tau_{r1}, \tau_{r2}) \qquad
  \text{unify}(\rho_{q1}, \rho_{q2}) \qquad
  \text{unify}(\rho_{e1}, \rho_{e2})
  \end{array}
}{
  \text{unify}((\overline{p_1}) \to \tau_{r1};\, \rho_{q1};\, \rho_{e1},\;
  (\overline{p_2}) \to \tau_{r2};\, \rho_{q2};\, \rho_{e2})
} \;\textsc{U-Arrow}$$

where $\pi$ is the permutation matching labels by name ($\ell_i^1 = \ell_{\pi(i)}^2$). Parameters are matched by label, not position.

$$\dfrac{
  \lvert\overline{f_1}\rvert = \lvert\overline{f_2}\rvert \qquad
  \text{sorted by label} \qquad
  \forall i.\;\ell_i^1 = \ell_i^2 \qquad
  \forall i.\;\text{unify}(\tau_i^1, \tau_i^2)
}{
  \text{unify}(\lbrace \overline{f_1} \rbrace, \lbrace \overline{f_2} \rbrace)
} \;\textsc{U-Record}$$

$$\dfrac{
  \forall i.\;\text{unify}(\tau_i, \sigma_i)
}{
  \text{unify}(x\langle\overline{\tau}\rangle, x\langle\overline{\sigma}\rangle)
} \;\textsc{U-Named}$$

$$\dfrac{
  \begin{array}{l}
  \overline{c} = \overline{\tau_1} \cap \overline{\tau_2} \qquad
  \overline{r_1} = \overline{\tau_1} \setminus \overline{c} \qquad
  \overline{r_2} = \overline{\tau_2} \setminus \overline{c} \\[4pt]
  {?}\gamma ~\text{fresh} \qquad
  \text{unify}({?}t_1, \lbrace \overline{r_2} \mid {?}\gamma \rbrace) \qquad
  \text{unify}({?}t_2, \lbrace \overline{r_1} \mid {?}\gamma \rbrace)
  \end{array}
}{
  \text{unify}(\lbrace \overline{\tau_1} \mid {?}t_1 \rbrace, \lbrace \overline{\tau_2} \mid {?}t_2 \rbrace)
} \;\textsc{U-Row}$$

$$\dfrac{
  \text{unify}(\tau_1, \tau_2)
}{
  \text{unify}(\&\tau_1, \tau_2)
} \;\textsc{U-Borrow}$$

$$\dfrac{
  \text{fields}(x\langle\overline{\tau}\rangle) = \lbrace \overline{\ell : \sigma} \rbrace \qquad
  \text{unify}(\lbrace \overline{\ell : \sigma} \rbrace, R)
}{
  \text{unify}(x\langle\overline{\tau}\rangle, R)
} \;\textsc{U-Expand}$$

U-Borrow is intentionally asymmetric (auto-derefs only the left argument). U-Expand applies in both argument orders via the symmetry convention — the implementation handles both $\text{unify}(x\langle\overline{\tau}\rangle, R)$ and $\text{unify}(R, x\langle\overline{\tau}\rangle)$.

$$\textbf{P7}~\text{(Unification).}\quad \text{unify}(\tau_1, \tau_2)~\text{terminates and returns a most general unifier or fails}$$

### Generalization and Instantiation

$\text{gen}$ and $\text{inst}$ are functions used in [T-Let](#T-Let) and [T-Var](#T-Var), not inference rules.

$$\text{gen}(\Gamma, \tau) = \forall \alpha_1 \ldots \alpha_n.\, \tau \qquad\text{where } \lbrace \alpha_1, \ldots, \alpha_n \rbrace = \text{fv}(\tau) \setminus \text{fv}(\Gamma)$$

$\text{fv}(\tau)$ returns free unification variables in $\tau$. $\text{fv}(\Gamma)$ returns the union of free variables in all schemes in $\Gamma$, excluding each scheme's bound variables.

$$\text{inst}(\forall \alpha_1 \ldots \alpha_n.\, \tau) = \tau[\alpha_1 := {?}\beta_1, \ldots, \alpha_n := {?}\beta_n] \qquad (\overline{{?}\beta}~\text{fresh})$$

$$\textbf{P8}~\text{(Generalization).}\quad \text{gen}(\Gamma, \tau)~\text{quantifies exactly}~\text{fv}(\tau) \setminus \text{fv}(\Gamma)$$

### Pattern Matching

$$\Gamma \vdash p : \tau \Rightarrow \Gamma'$$

$\text{strip}$ removes the outermost modality before pattern matching. It does not remove $\mathord{\sim}$ (mutable ref) — refs cannot be match scrutinees.

$$\text{strip}(\tau) = \begin{cases}
\sigma & \text{if } \tau \in \lbrace \%\sigma,\, @\sigma,\, \&\sigma \rbrace \\
\tau & \text{otherwise}
\end{cases}$$

The match expression ([T-Match](#T-Match)) consumes the linear scrutinee via $\otimes$; the pattern rules operate on the stripped type.

$$\dfrac{
  q = \begin{cases} 1 & \text{if } \text{linear}(\tau) \\ \omega & \text{otherwise} \end{cases}
}{
  \Gamma \vdash x : \tau \Rightarrow \Gamma,\, x :^{q} (\emptyset, \tau)
} \;\textsc{P-Var}$$

$$\dfrac{
  \text{linear}(\tau) \wedge \neg\text{autoDrop}(\tau) \implies \text{error}
}{
  \Gamma \vdash \_ : \tau \Rightarrow \Gamma
} \;\textsc{P-Wild}$$

$$\dfrac{
  \text{unify}(\tau, \text{typeof}(n))
}{
  \Gamma \vdash n : \tau \Rightarrow \Gamma
} \;\textsc{P-Lit}$$

$$\dfrac{
  \begin{array}{l}
  \Gamma(c) = \forall\overline{\alpha}.\,(\overline{\ell : F}) \to \tau' \\[2pt]
  \text{unify}(\text{strip}(\tau),\, \tau'[\overline{\alpha := {?}\beta}]) \qquad
  \lvert\overline{p}\rvert = \lvert\overline{F}\rvert \\[2pt]
  \Gamma \vdash p_1 : F_1[\overline{\alpha := {?}\beta}] \Rightarrow \Gamma_1 \qquad \ldots \qquad \Gamma_{k-1} \vdash p_k : F_k[\overline{\alpha := {?}\beta}] \Rightarrow \Gamma_k
  \end{array}
}{
  \Gamma \vdash c(\overline{\ell : p}) : \tau \Rightarrow \Gamma_k
} \;\textsc{P-Ctor}$$

Field patterns are checked sequentially, threading the environment: each sub-pattern extends $\Gamma$ with new bindings that subsequent sub-patterns can see.

$$\dfrac{
  \begin{array}{l}
  \forall i.\;\tau_i = \text{fields}(\text{strip}(\tau)).\ell_i \\[2pt]
  \Gamma \vdash p_1 : \tau_1 \Rightarrow \Gamma_1 \qquad \ldots \qquad \Gamma_{k-1} \vdash p_k : \tau_k \Rightarrow \Gamma_k
  \end{array}
}{
  \Gamma \vdash \lbrace \overline{\ell : p} \rbrace : \tau \Rightarrow \Gamma_k
} \;\textsc{P-Record}$$

Exhaustiveness is checked via Maranget's pattern matrix algorithm:

$$\dfrac{M = \emptyset}{\text{check}(M, \overline{\tau}) = \text{error}} \;\textsc{Exh-Empty}
\qquad
\dfrac{M \neq \emptyset \qquad \lvert\overline{\tau}\rvert = 0}{\text{check}(M, \overline{\tau}) = \text{ok}} \;\textsc{Exh-Done}$$

$$\dfrac{
  \text{check}(\text{spec}(M, \texttt{true}), \overline{\tau}') \qquad
  \text{check}(\text{spec}(M, \texttt{false}), \overline{\tau}')
}{
  \text{check}(M, \texttt{bool} :: \overline{\tau}')
} \;\textsc{Exh-Bool}$$

$$\dfrac{
  \begin{array}{l}
  \text{variants}(\tau_1) = \overline{c_j(\overline{F_j})} \\[2pt]
  \forall j.\;\text{check}(\text{spec}(M, c_j), \overline{F_j} \mathbin{+\!\!+} \overline{\tau}')
  \end{array}
}{
  \text{check}(M, \tau_1 :: \overline{\tau}')
} \;\textsc{Exh-Sum}$$

$$\dfrac{
  \begin{array}{l}
  \text{fields}(\tau_1) = \overline{\ell : \sigma} \\[2pt]
  \text{check}(\text{spec}_R(M, \overline{\ell}), \overline{\sigma} \mathbin{+\!\!+} \overline{\tau}')
  \end{array}
}{
  \text{check}(M, \tau_1 :: \overline{\tau}')
} \;\textsc{Exh-Record}$$

$\text{spec}_R(M, \overline{\ell})$ specializes the matrix for a record scrutinee: rows whose first pattern is $\lbrace \overline{\ell : p} \rbrace$ contribute $\overline{p}$ prepended to the rest; wildcard rows are replicated with $\lvert\overline{\ell}\rvert$ fresh wildcards.

$$\dfrac{
  D = \lbrace\, \overline{r} \mid (\_ :: \overline{r}) \in M \,\rbrace \qquad
  \text{check}(D, \overline{\tau}')
}{
  \text{check}(M, \tau_1 :: \overline{\tau}')
} \;\textsc{Exh-Default}$$

Exh-Default applies when the first column has no complete constructor coverage — it computes the default matrix $D$ by collecting rows whose first pattern is a wildcard or variable.

$$\text{spec}(M, c) = \lbrace\, \overline{p'} \mathbin{+\!\!+} \overline{r} \mid (c(\overline{p'}) :: \overline{r}) \in M \,\rbrace \;\cup\; \lbrace\, \underbrace{\_,\ldots,\_}_{a(c)} \mathbin{+\!\!+} \overline{r} \mid (\_ :: \overline{r}) \in M \,\rbrace$$

where $a(c)$ is the arity of constructor $c$. Rows whose first pattern is $c(\overline{p'})$ contribute $\overline{p'}$ prepended to the rest; wildcard rows are replicated with $a(c)$ fresh wildcards.

$$\textbf{P6}~\text{(Exhaustiveness).}\quad \text{check}(M, [\tau]) = \text{ok} \implies \forall v : \tau.\;\exists i.\; v \in \text{match}(p_i)$$

### Expressions

$$\Gamma;\, \rho_q \vdash_e e : \tau \mathbin{!} \rho_e$$

All linear bindings in $\Gamma$ must be consumed by the derivation; $\otimes$ distributes them among sub-expressions. $\rho_e$ ($\mathbin{!}$) is the effect produced. Literal rules require $\Gamma$ to contain no linear bindings.

$$\dfrac{}{\Gamma;\, \rho_q \vdash_e n : \texttt{intlit} \mathbin{!} \lbrace\rbrace} \;\textsc{T-IntLit}
\qquad
\dfrac{}{\Gamma;\, \rho_q \vdash_e f : \texttt{floatlit} \mathbin{!} \lbrace\rbrace} \;\textsc{T-FloatLit}$$

$$\dfrac{}{\Gamma;\, \rho_q \vdash_e b : \texttt{bool} \mathbin{!} \lbrace\rbrace} \;\textsc{T-Bool}
\qquad
\dfrac{}{\Gamma;\, \rho_q \vdash_e ch : \texttt{char} \mathbin{!} \lbrace\rbrace} \;\textsc{T-Char}$$

$$\dfrac{}{\Gamma;\, \rho_q \vdash_e s : \texttt{string} \mathbin{!} \lbrace\rbrace} \;\textsc{T-Str}
\qquad
\dfrac{}{\Gamma;\, \rho_q \vdash_e () : \texttt{unit} \mathbin{!} \lbrace\rbrace} \;\textsc{T-Unit}$$

<a id="T-Var"></a>

$$\dfrac{
  x :^{q} \forall\overline{\alpha}.\,\tau \in \Gamma \qquad
  \tau' = \text{inst}(\forall\overline{\alpha}.\,\tau) \qquad
  \Gamma \setminus \lbrace x \rbrace ~\text{has no linear bindings}
}{
  \Gamma;\, \rho_q \vdash_e x : \tau' \mathbin{!} \lbrace\rbrace
} \;\textsc{T-Var}$$

If $q = 1$, the binding $x$ is consumed by this use. For $\mathord{\sim}x$: if $\tau' = \mathord{\sim}\sigma$, the expression has type $\sigma$ (dereference). For $@x$: if $\tau' = @\sigma$, the expression has type $\sigma$ (see also [T-Force](#T-Force)).

To allow functions with fewer capabilities/effects to be called in a context with more (row subsumption in [T-App](#T-App)), we introduce $\text{open}$:

$$\text{open}(\rho) = \begin{cases}
\lbrace \overline{\tau} \mid {?}r \rbrace & \text{if } \rho = \lbrace \overline{\tau} \rbrace,\; {?}r~\text{fresh} \\
\rho & \text{if } \rho = \lbrace \overline{\tau} \mid {?}r \rbrace
\end{cases}$$

<a id="T-App"></a>

$$\dfrac{
  \begin{array}{c}
  \Gamma = \Gamma_f \otimes \Gamma_1 \otimes \ldots \otimes \Gamma_k \\[2pt]
  \Gamma_f;\, \rho_q \vdash_e f : (\overline{\ell : P}) \to \tau_r;\, \rho_q';\, \rho_e' \mathbin{!} \rho_f \\[2pt]
  \forall i.\;\Gamma_i;\, \rho_q \vdash_e e_i : \tau_i \mathbin{!} \rho_i \\[2pt]
  \forall i.\;\begin{cases} \text{unify}(\tau_i, \text{strip}(P_i)) & \text{if } P_i = \%\sigma \wedge \neg\text{linear}(\tau_i) \\ \text{unify}(\tau_i, P_i) & \text{otherwise} \end{cases} \\[2pt]
  \text{unify}(\rho_q, \text{open}(\rho_q'))
  \end{array}
}{
  \Gamma;\, \rho_q \vdash_e f(\overline{\ell : e}) : \tau_r \mathbin{!} \rho_e' \cup \rho_f \cup \textstyle\bigcup_i \rho_i
} \;\textsc{T-App}$$

The weakening condition is explicit: when $P_i = \%\sigma$ and $\tau_i$ is not linear, unification targets the inner type $\sigma$ (stripping the $\%$ wrapper). This does not apply to other linear forms like $@\sigma$.

$\text{selectInt}$ and $\text{selectFloat}$ resolve operand types. Integer and float operators are separate ($+$ vs $+.$); they cannot mix. "$—$" = type error (no implicit coercion).

$$\text{selectInt}: \quad
\begin{array}{c|ccc}
 & \texttt{intlit} & \texttt{i32} & \texttt{i64} \\
\hline
\texttt{intlit} & \texttt{i64} & \texttt{i32} & \texttt{i64} \\
\texttt{i32} & \texttt{i32} & \texttt{i32} & — \\
\texttt{i64} & \texttt{i64} & — & \texttt{i64}
\end{array}
$$

$$\text{selectFloat}: \quad
\begin{array}{c|ccc}
 & \texttt{floatlit} & \texttt{f32} & \texttt{f64} \\
\hline
\texttt{floatlit} & \texttt{f64} & \texttt{f32} & \texttt{f64} \\
\texttt{f32} & \texttt{f32} & \texttt{f32} & — \\
\texttt{f64} & \texttt{f64} & — & \texttt{f64}
\end{array}$$

$$\dfrac{
  \begin{array}{l}
  \Gamma = \Gamma_1 \otimes \Gamma_2 \\[2pt]
  \Gamma_1;\, \rho_q \vdash_e e_1 : \tau_1 \mathbin{!} \rho_1 \qquad
  \Gamma_2;\, \rho_q \vdash_e e_2 : \tau_2 \mathbin{!} \rho_2 \\[2pt]
  \tau = \text{selectInt}(\tau_1, \tau_2) \qquad
  \text{unify}(\tau_1, \tau) \qquad
  \text{unify}(\tau_2, \tau)
  \end{array}
}{
  \Gamma;\, \rho_q \vdash_e e_1 \oplus e_2 : \tau \mathbin{!} \rho_1 \cup \rho_2
} \;\textsc{T-ArithInt}$$

$$\dfrac{
  \begin{array}{l}
  \Gamma = \Gamma_1 \otimes \Gamma_2 \\[2pt]
  \Gamma_1;\, \rho_q \vdash_e e_1 : \tau_1 \mathbin{!} \rho_1 \qquad
  \Gamma_2;\, \rho_q \vdash_e e_2 : \tau_2 \mathbin{!} \rho_2 \\[2pt]
  \tau = \text{selectFloat}(\tau_1, \tau_2) \qquad
  \text{unify}(\tau_1, \tau) \qquad
  \text{unify}(\tau_2, \tau)
  \end{array}
}{
  \Gamma;\, \rho_q \vdash_e e_1 \oplus_f e_2 : \tau \mathbin{!} \rho_1 \cup \rho_2
} \;\textsc{T-ArithFloat}$$

Unresolved type variables (${?}\alpha$) are treated as $\texttt{intlit}$ in $\text{selectInt}$ and as $\texttt{floatlit}$ in $\text{selectFloat}$.

Integer operators ($+$, $-$, $*$, etc.) use $\text{selectInt}$; float operators ($+.$, $-.$, etc.) use $\text{selectFloat}$. The two cannot mix.

$$\dfrac{
  \begin{array}{l}
  \Gamma = \Gamma_c \otimes \Gamma_b \\[2pt]
  \Gamma_c;\, \rho_q \vdash_e e_c : \tau_c \mathbin{!} \rho_c \qquad
  \text{unify}(\tau_c, \texttt{bool}) \\[2pt]
  \Gamma_b;\, \rho_q;\, \tau_r \vdash_s \overline{s_1} : \Gamma_1' \mathbin{!} \rho_1 \\[2pt]
  \Gamma_b;\, \rho_q;\, \tau_r \vdash_s \overline{s_2} : \Gamma_2' \mathbin{!} \rho_2
  \end{array}
}{
  \Gamma;\, \rho_q \vdash_e \textbf{if}~e_c~\textbf{then}~\overline{s_1}~\textbf{else}~\overline{s_2} : \texttt{unit} \mathbin{!} \rho_c \cup \rho_1 \cup \rho_2
} \;\textsc{T-If}$$

Both branches receive the **same** $\Gamma_b$ (since only one executes at runtime).

To unify the result types of match arms ([T-Match](#T-Match)), we introduce $\text{tail}$, which extracts the type produced by the last statement in a sequence:

$$\text{tail}(\overline{s}) = \begin{cases} \tau & \text{if last statement is an expression of type } \tau \\ \texttt{unit} & \text{if last statement is } \textbf{let},\; \mathord{\sim}x \leftarrow e,\; \textbf{inject},\; \textbf{try}\text{-}\textbf{catch} \\ \bot & \text{if last statement is } \textbf{return},\text{ or an expression statement whose expression is } \textbf{raise}~e \end{cases}$$

<a id="T-Match"></a>

$$\dfrac{
  \begin{array}{l}
  \Gamma = \Gamma_e \otimes \Gamma_b \\[2pt]
  \Gamma_e;\, \rho_q \vdash_e e : \tau \mathbin{!} \rho_0 \qquad
  \text{exhaustive}(\text{strip}(\tau), \overline{p}) \\[4pt]
  \forall i.\;\Gamma_b \vdash p_i : \text{strip}(\tau) \Rightarrow \Gamma_i \\[2pt]
  \forall i.\;\Gamma_i;\, \rho_q;\, \tau_r \vdash_s \overline{s_i} : \Gamma_i' \mathbin{!} \rho_i \\[4pt]
  \forall i, j.\;\text{tail}(\overline{s_i}) \neq \bot \wedge \text{tail}(\overline{s_j}) \neq \bot \implies \text{unify}(\text{tail}(\overline{s_i}), \text{tail}(\overline{s_j}))
  \end{array}
}{
  \Gamma;\, \rho_q \vdash_e \textbf{match}~e~\lbrace \overline{p_i \to s_i} \rbrace : \sigma \mathbin{!} \rho_0 \cup \textstyle\bigcup_i \rho_i
} \;\textsc{T-Match}$$

where $\sigma$ is the common type of non-diverging ($\text{tail} \neq \bot$) arms. If all arms diverge ($\forall i.\;\text{tail}(\overline{s_i}) = \bot$), $\sigma$ is a fresh ${?}\alpha$. All arms receive the same $\Gamma_b$.

<a id="T-Lambda"></a>

$$\dfrac{
  \begin{array}{l}
  \Gamma_\text{cap} = \lbrace x :^{1} S \in \Gamma \mid x \in \text{fv}(\overline{s}) \rbrace \\[2pt]
  \Gamma_\omega = \lbrace x :^{\omega} S \in \Gamma \mid x \in \text{fv}(\overline{s}) \rbrace \\[2pt]
  \forall x \in \text{fv}(\overline{s}) \cap \text{dom}(\Gamma).\;\Gamma(x) \neq \mathord{\sim}\sigma \quad\text{(no ref capture)} \\[2pt]
  \Gamma_\omega,\, \Gamma_\text{cap},\, \overline{x_i :^{\omega} \tau_i};\, \rho_q;\, \tau_r \vdash_s \overline{s} : \Gamma' \mathbin{!} \rho_e
  \end{array}
}{
  \Gamma_\text{cap};\, \rho_q' \vdash_e \textbf{fn}~(\overline{\ell : \tau}) \to \tau_r;\, \rho_q;\, \rho_e~\textbf{do}~\overline{s}~\textbf{end} : \tau_\to \mathbin{!} \lbrace\rbrace
} \;\textsc{T-Lambda}$$

The lambda is pure ($\mathbin{!} \lbrace\rbrace$). It consumes $\Gamma_\text{cap}$ (captured linear bindings). The body environment includes $\Gamma_\omega$ (captured unrestricted bindings), $\Gamma_\text{cap}$, and the parameters $\overline{x_i :^{\omega} \tau_i}$. Body effects become part of $\tau_\to = (\overline{\ell : \tau}) \to \tau_r;\, \rho_q;\, \rho_e$. If $\Gamma_\text{cap} \neq \emptyset$, $\tau_\to$ becomes $\%\tau_\to$.

$$\dfrac{
  \Gamma;\, \rho_q \vdash_e e : \tau \mathbin{!} \rho_0 \qquad
  \text{unify}(\tau, \texttt{Exn})
}{
  \Gamma;\, \rho_q \vdash_e \textbf{raise}~e : {?}\alpha \mathbin{!} \lbrace \texttt{Exn} \rbrace \cup \rho_0
} \;\textsc{T-Raise}$$

$$\dfrac{
  x :^{\omega} \forall\overline{\alpha}.\,\tau \in \Gamma \qquad
  \tau' = \text{inst}(\forall\overline{\alpha}.\,\tau) \qquad
  \sigma = \text{strip}(\tau')
}{
  \Gamma;\, \rho_q \vdash_e \&x : \&\sigma \mathbin{!} \lbrace\rbrace
} \;\textsc{T-Borrow}$$

Borrowing does not consume the binding. Only unrestricted bindings can be borrowed.

<a id="T-Force"></a>

$$\dfrac{
  \Gamma;\, \rho_q \vdash_e e : @\sigma \mathbin{!} \rho_0
}{
  \Gamma;\, \rho_q \vdash_e @e : \sigma \mathbin{!} \rho_0
} \;\textsc{T-Force}$$

The thunk is consumed via [T-Var](#T-Var) ($q = 1$) in the sub-derivation.

$$\dfrac{
  \Gamma = \Gamma_1 \otimes \ldots \otimes \Gamma_k \qquad
  \forall i.\;\Gamma_i;\, \rho_q \vdash_e e_i : \tau_i \mathbin{!} \rho_i
}{
  \Gamma;\, \rho_q \vdash_e \lbrace \overline{\ell : e} \rbrace : \lbrace \overline{\ell : \tau_i} \rbrace \mathbin{!} \textstyle\bigcup_i \rho_i
} \;\textsc{T-Record}$$

$$\dfrac{
  \Gamma;\, \rho_q \vdash_e e : \tau \mathbin{!} \rho_0 \qquad
  (\ell : \sigma) \in \text{fields}(\tau)
}{
  \Gamma;\, \rho_q \vdash_e e.\ell : \sigma \mathbin{!} \rho_0
} \;\textsc{T-Proj}$$

### Statements

$$\Gamma;\, \rho_q;\, \tau_r \vdash_s s : \Gamma' \mathbin{!} \rho_e$$

$\Gamma'$ is the updated environment (new bindings from [T-Let](#T-Let)). $\tau_r$ is the expected return type of the enclosing function.

[T-Let](#T-Let) resolves numeric literals and applies sigil modalities via two auxiliaries. $\text{default}$ defaults $\texttt{intlit}$/$\texttt{floatlit}$ at binding sites where no concrete type was inferred:

$$\text{default}(\tau) = \begin{cases}
\texttt{i64} & \text{if } \tau = \texttt{intlit} \\
\texttt{f64} & \text{if } \tau = \texttt{floatlit} \\
\tau & \text{otherwise (applied recursively to all subterms)}
\end{cases}$$

$\text{wrapSigil}$ wraps the inferred type with the modality corresponding to the binding's sigil:

$$\text{wrapSigil}(\mu, \tau) = \begin{cases}
\%\tau & \text{if } \mu = \% \\
\mathord{\sim}\tau & \text{if } \mu = \mathord{\sim} \\
@\tau & \text{if } \mu = @ \\
\&\tau & \text{if } \mu = \& \\
\tau & \text{if } \mu = \varepsilon
\end{cases}$$

<a id="T-Let"></a>

$$\dfrac{
  \begin{array}{l}
  \Gamma;\, \rho_q \vdash_e e : \tau \mathbin{!} \rho_0 \\[2pt]
  \tau' = \text{default}(\tau) \\[2pt]
  \tau_f = \text{wrapSigil}(\mu, \tau') \qquad
  S = \text{gen}(\Gamma, \tau_f) \\[2pt]
  q = \begin{cases} 1 & \text{if } \text{linear}(\tau_f) \\ \omega & \text{otherwise} \end{cases}
  \end{array}
}{
  \Gamma;\, \rho_q;\, \tau_r \vdash_s \textbf{let}~\mu\,x = e : \Gamma,\, x :^{q} S \mathbin{!} \rho_0
} \;\textsc{T-Let}$$

When the surface syntax includes a type annotation ($\textbf{let}~\mu\,x : \sigma = e$), an additional premise $\text{unify}(\tau', \sigma)$ is required and $\tau'$ is replaced by $\sigma$. When the annotation is absent, $\tau'$ remains as inferred (possibly containing unification variables that are resolved later or defaulted).

$$\dfrac{
  \Gamma;\, \rho_q \vdash_e e : \tau \mathbin{!} \rho_0 \qquad
  \text{unify}(\tau, \tau_r)
}{
  \Gamma;\, \rho_q;\, \tau_r \vdash_s \textbf{return}~e : \Gamma \mathbin{!} \rho_0
} \;\textsc{T-Return}$$

$$\dfrac{
  x :^{\omega} (\emptyset,\, \mathord{\sim}\tau) \in \Gamma \qquad
  \Gamma;\, \rho_q \vdash_e e : \sigma \mathbin{!} \rho_0 \qquad
  \text{unify}(\sigma, \tau)
}{
  \Gamma;\, \rho_q;\, \tau_r \vdash_s \mathord{\sim}x \leftarrow e : \Gamma \mathbin{!} \rho_0
} \;\textsc{T-Assign}$$

[T-Inject](#T-Inject) combines capability rows from injected handlers via two auxiliaries. $\text{merge}$ unions two rows:

$$\text{merge}(\rho_1, \rho_2) = \rho_1 \cup \rho_2 \quad\text{(row union, deduplicating by type identity)}$$

$\text{ports}$ collects the capability names that a set of handlers provides:

$$\text{ports}(\overline{\rho}) = \textstyle\bigcup_i \lbrace\, P \in \rho_i \mid P~\text{is a capability name} \,\rbrace$$

<a id="T-Inject"></a>

$$\dfrac{
  \begin{array}{l}
  \forall i.\;\text{inst}(\Gamma(h_i)) = \textbf{handler}\;P_i\;\rho_i \\[2pt]
  \rho_q' = \text{merge}(\rho_q,\, \lbrace \overline{P} \rbrace \cup \text{ports}(\overline{\rho_i})) \\[2pt]
  \Gamma;\, \rho_q';\, \tau_r \vdash_s \overline{s} : \Gamma' \mathbin{!} \rho_0
  \end{array}
}{
  \Gamma;\, \rho_q;\, \tau_r \vdash_s \textbf{inject}~\overline{h}~\textbf{do}~\overline{s}~\textbf{end} : \Gamma' \mathbin{!} \rho_0
} \;\textsc{T-Inject}$$

$$\dfrac{
  \begin{array}{l}
  \Gamma;\, \rho_q;\, \tau_r \vdash_s \overline{s_\text{try}} : \Gamma_1 \mathbin{!} \rho_\text{try} \\[4pt]
  \forall i.\;\Gamma_1 \vdash p_i : \texttt{Exn} \Rightarrow \Gamma_i \\[2pt]
  \forall i.\;\Gamma_i;\, \rho_q;\, \tau_r \vdash_s \overline{s_i} : \Gamma_i' \mathbin{!} \rho_i
  \end{array}
}{
  \Gamma;\, \rho_q;\, \tau_r \vdash_s \textbf{try}~\overline{s_\text{try}}~\textbf{catch}~\overline{p_i \to s_i}~\textbf{end} : \Gamma_1 \mathbin{!} (\rho_\text{try} \setminus \lbrace\texttt{Exn}\rbrace) \cup \textstyle\bigcup_i \rho_i
} \;\textsc{T-TryCatch}$$

The output environment is $\Gamma_1$ (from the try block). The catch arms extend $\Gamma_1$ with pattern bindings but the output of the whole construct is $\Gamma_1$.

$$\dfrac{
  \Gamma;\, \rho_q \vdash_e e : \tau \mathbin{!} \rho_0 \qquad
  \text{exhaustive}(\tau, [p]) \qquad
  \Gamma \vdash p : \tau \Rightarrow \Gamma'
}{
  \Gamma;\, \rho_q;\, \tau_r \vdash_s \textbf{let}~p = e : \Gamma' \mathbin{!} \rho_0
} \;\textsc{T-LetPat}$$

{% endraw %}
