# Subreddit Registry — Humla

Single source of truth for which subs the routines target. Every routine (karma-builder, lead-finder, research-and-drafts, historical-scan) reads from this file instead of duplicating sub lists.

When you find a new sub worth tracking, add it here. The next routine run will pick it up.

---

## Tier 1 — Core target subs (high fit, regular monitoring)

These are Humla's primary audience. Routines should always include these.

### r/macapps
- **Subscribers:** ~225k
- **Karma gate:** **10 local karma to post** (verified from rules JSON)
- **Promo rules:** 1 dev post / 30 days; flair required (`[App]`, `[OS]` if open source); disclose maker status; comments promoting your app banned until 10 local karma
- **Why fit:** Mac-only, primary audience, Humla is a Mac app. Top posts are Problem→Comparison→Solution prose with inline GIF/v.redd.it.
- **Query patterns (lead-finder):** `"meeting notes"`, `"transcription"`, `"granola"`, `"otter"`, `"fathom"`, `"fireflies"`, `"notion ai"`, `"system audio"`, `"AI note taker"`, `"local"`, `"offline"`
- **Status:** locked until karma ≥10; karma-builder primary target
- **Special:** [The App Pile megathread](https://reddit.com/r/macapps/comments/1t0rojv/megathread_the_app_pile_may_2026/) is a stickied monthly thread for non-MAS / non-qualified apps to promote — usable now without main-feed qualification

### r/AiNoteTaker
- **Subscribers:** small (low traffic but high intent)
- **Karma gate:** none published
- **Promo rules:** lenient; disclose maker
- **Why fit:** dedicated to the exact category Humla competes in. Threads here are almost always buying intent.
- **Query patterns (lead-finder):** `"alternative"`, `"local"`, `"offline"`, `"open source"`, `"FOSS"`, `"privacy"`, `"mac"`, `"no bot"`, `"speaker identification"`, `"notion"`, `"too expensive"`
- **Status:** unlocked. Michael already has 1 Humla mention here ([5/1/2026 thread](https://reddit.com/r/AiNoteTaker/comments/1sxue3y))
- **Cadence note:** can go 24–72h without new asks; lead-finder uses 72h window for this sub

### r/AI_Agents
- **Subscribers:** ~353k
- **Karma gate:** none (verified from rules JSON 2026-05-02)
- **Promo rules:** Self-promo allowed at 1/10 ratio; **links go in comments, not posts**; no spam; no low-effort posts. Disclose maker status.
- **Why fit:** Bucket A intent threads land here regularly ("Is there an AI note taker for in-person meetings?", "Are we still stuck reviewing AI meeting notes in 2025?"). Audience is buying-mode for meeting tools. **Promoted from Tier 2 → Tier 1 after producing 2 Bucket A leads on first historical scan (2026-05-02).**
- **Query patterns (lead-finder):** `"meeting"`, `"note taker"`, `"in person"`, `"transcription"`, `"on-device"`, `"speaker"`, `"Granola alternative"`, `"Notion AI"`, `"too expensive"`
- **Status:** unlocked, verified-allowed

### r/MacOS
- **Subscribers:** large
- **Karma gate:** none published
- **Promo rules:** **self-promo Saturdays UTC only**; needs Mac App Store or "reputable, established GitHub repository"; GitHub-Guard auditing
- **Why fit:** broader Mac audience; Humla qualifies via public GitHub repo
- **Query patterns (lead-finder):** `"transcribe"`, `"meeting"`, `"system audio"`, `"screen audio"`
- **Status:** unlocked but Saturday-only for self-promo; karma-building any day

### r/SideProject
- **Subscribers:** ~700k
- **Karma gate:** none (rules JSON empty)
- **Promo rules:** very lenient; disclose maker
- **Why fit:** indie/builder audience, lenient rules; good for first launch posts and "building in public" updates
- **Query patterns (lead-finder):** `"meeting notes"`, `"transcription"`, `"granola"`
- **Status:** unlocked; Michael has karma here

### r/sideprojects (lowercase, separate sub)
- Same shape as r/SideProject; lenient
- **Status:** unlocked; Michael has karma here

### r/buildinpublic
- **Karma gate:** none published
- **Promo rules:** lenient; disclose
- **Why fit:** builder narrative posts; Humla's "built in Norway under EU privacy frame" angle fits perfectly
- **Query patterns (lead-finder):** `"meeting notes"`, `"transcription"`
- **Status:** unlocked; Michael has karma here

### r/LocalLLaMA
- **Subscribers:** ~710k
- **Karma gate:** none published
- **Promo rules:** 1/10 rule; **NO LLM-generated content** — every post and comment must be hand-written; affiliation must be disclosed
- **Why fit:** whisper / on-device transcription / local AI crowd. Humla uses whisper-rs + Metal + FluidAudio CoreML — all on-topic.
- **Query patterns (lead-finder):** `"whisper meeting"`, `"transcription local"`, `"diarization"`, `"meeting notes"`, `"voxtral"`, `"parakeet meeting"`
- **Status:** unlocked; high quality bar; **drafts must be hand-written, do not paste AI output**

### r/ClaudeCode
- **Karma gate:** none published
- **Promo rules:** lenient; disclose
- **Why fit:** Humla is built with Claude Code — natural authentic angle. Audience is technical, appreciates skill/tool sharing.
- **Query patterns (lead-finder):** `"transcription"`, `"meeting"`, `"whisper"`
- **Status:** unlocked; Michael has karma here. Audience currently dominated by Opus 4.7 limit complaints — buying mood is low, but skill/tool sharing posts (like `humanizer`) hit 500+ upvotes

### r/ClaudeAI
- Same as r/ClaudeCode; broader audience
- **Status:** unlocked; Michael has karma here

---

## Tier 2 — Adjacent (Mac dev / on-device AI / privacy / open source)

Lower-frequency monitoring; surfaces fewer leads but worth covering in research + historical-scan.

### r/IMadeThis
- **Subscribers:** ~30k
- **Karma gate:** none published
- **Promo rules:** lenient; meant for showing what you built
- **Why fit:** alt to r/SideProject; competitor Myna posted there
- **Query patterns:** `"meeting"`, `"transcription"`, `"mac"`
- **Status:** unlocked

### r/indiehackers
- **Subscribers:** ~167k
- **Karma gate:** none published
- **Promo rules:** lenient with self-promo flair
- **Why fit:** bootstrap/builder audience overlapping with buildinpublic
- **Status:** unlocked; verify rules before posting

### r/microsaas
- **Karma gate:** unverified
- **Promo rules:** unverified; likely lenient
- **Status:** unverified — verify on next routine run

### r/opensource
- **Karma gate:** unverified
- **Why fit:** Humla repo is on GitHub; OSS framing fits
- **Status:** unverified

### r/freesoftware
- **Karma gate:** unverified
- **Why fit:** Humla is free + source-available; aligns with FOSS values (caveat: not strictly FSF-free if it depends on closed components)
- **Status:** unverified

### r/Tauri
- **Karma gate:** unverified (likely none, small sub)
- **Why fit:** Humla is built on Tauri 2; technical posts about Tauri sidecar architecture, ScreenCaptureKit integration, signing pipeline are genuinely useful here
- **Status:** unverified — high signal if active

### r/swift
- **Karma gate:** unverified
- **Why fit:** Humla's audio-capture and speaker-diarize sidecars are Swift; ScreenCaptureKit + AVAudioEngine + FluidAudio integration story
- **Status:** unverified

### r/rust
- **Karma gate:** unverified
- **Why fit:** Tauri + whisper-rs backend; FFI + audio pipeline interesting to Rustaceans
- **Status:** unverified

### r/macprogramming
- **Karma gate:** unverified
- **Why fit:** Mac dev community; signing/notarization/TCC posts on-topic
- **Status:** unverified

### r/ProductivityApps
- **Karma gate:** unverified
- **Why fit:** "Top apps" list-style posts surface here (Granola was named in one such post)
- **Status:** unverified

### r/Notion
- **Karma gate:** unverified
- **Why fit:** Notion's AI Meeting Notes is a direct Humla competitor. Notion's pricing model is the personal pain point that drove Humla's existence — team plan to keep AI features ran into 4-figure subscription paywall. People hitting the same wall surface here regularly. High-intent for the BYO-key + local-first frame.
- **Query patterns (lead-finder):** `"AI meeting notes"`, `"transcription"`, `"too expensive"`, `"trial ended"`, `"alternative"`, `"team plan"`, `"voice"`
- **Status:** unverified — verify rules on first encounter; treat as engagement-only until verified-allowed

---

## Tier 3 — Vertical (specific use cases)

Lower priority; good for targeted outreach if a thread is a strong fit.

### r/consulting
- **Karma gate:** unverified, likely strict
- **Promo rules:** Rule 5 = no spam (ads, free offers, market research, blogs, AI slop) — **effectively no self-promo**
- **Why fit:** consultants take meeting notes; high-intent audience but locked behind no-promo rules
- **Status:** **engagement-only** (no Humla mention)

### r/sales
- **Karma gate:** unverified
- **Why fit:** salespeople are heavy meeting-recorder users (Otter/Gong/Chorus territory)
- **Status:** unverified — likely strict; default to engagement-only until rules confirmed

### r/coaching
- **Karma gate:** unverified
- **Why fit:** coaches record sessions for review and notes
- **Status:** unverified

### r/UXResearch
- **Karma gate:** unverified
- **Why fit:** UX researchers transcribe interviews constantly
- **Status:** unverified — strong fit if rules allow

### r/RecruitmentAgencies
- **Karma gate:** unverified
- **Why fit:** Scout (a recruiter-vertical AI notetaker) posted there
- **Status:** unverified

### r/marketingagency
- **Karma gate:** unverified
- **Why fit:** marketers use meeting tools; Otter/Fellow/Gemini surfaced in workflow threads
- **Status:** unverified — workflow-integration audience, not Humla's natural fit

### r/ObsidianMD
- **Subscribers:** ~316k
- **Karma gate:** none published
- **Promo rules:** **first post = promo → instant ban**; no AI-generated content
- **Why fit:** Humla output is plain markdown → Obsidian-friendly; cross-pollinate as workflow post (NOT promo)
- **Status:** **engagement-only until Michael has real history in the sub**

---

## Tier 4 — Engagement-only (NO Humla mention, ever)

Comment for visibility/karma without pitching. Banned subs go here too.

### r/privacy
- **Subscribers:** ~1.6M
- **Promo rules:** Rule 3 = no advertising/marketing/products → **immediate ban without warning**
- **Why surface:** privacy threads name Otter/Fathom/Fireflies as data-leak risks. Humla's local-first frame is on-mission. Comment without product mention.
- **Status:** **engagement-only forever**

### r/BuyFromEU
- **Subscribers:** ~340k (founded 2025-02; politics/news heavy)
- **Promo rules:** unverified; sub is mostly EU-news + boycott discussions, not products
- **Why surface:** EU AI Act voice-data threads name US-server tools as liabilities; aligns with Humla's local-first + EU positioning. Worth tracking for context.
- **Status:** **engagement-only** (off-topic for direct promo); check threads where the EU-data-residency angle comes up

### r/productivity
- **Subscribers:** ~4.2M
- **Promo rules:** **bans AI-generated content; strict on self-promo**
- **Status:** **engagement-only**; rarely worth the effort given strictness

---

## Hard skips

These are explicitly OFF the routine list. Listed here so we don't accidentally re-add them.

- **r/selfhosted** — apps must be self-hosted server-style. Humla is local desktop. Posts get removed under Rule 1 ("Low-Effort / Off-Topic — Not Selfhosted"). Skip.

---

## How routines use this file

**karma-builder** — pulls priority sub list from Tier 1 (r/macapps, r/ClaudeCode, r/ClaudeAI, r/MacOS, r/LocalLLaMA, r/SideProject, r/sideprojects, r/buildinpublic) for daily browse. Reads "Status" field — skips locked subs for promo, uses them for karma-building only.

**lead-finder** — pulls per-sub query patterns from Tier 1 + 2. Skips Tier 4 (engagement-only) for promo-allowed surfacing but still surfaces them in the engagement-only section. Reads `Promo rules` field to set disclosure expectations.

**research-and-drafts** — Monday research scans Tier 1 + 2 for what's working in target subs. Friday draft picks `target_sub` from Tier 1 status field "unlocked" entries that match the topic.

**historical-scan** — sweeps the union of Tier 1 + 2 + 3 (60-day window). Tier 4 scanned for engagement-only candidates.

When a routine references a sub that's marked `Status: unverified`, it should attempt to fetch the rules JSON via curl + jq before posting/commenting and update this file with the verified answer. Routines should not invent karma gates that aren't in the rules JSON.

---

## Adding a new sub

Append to the appropriate Tier with this template:

```
### r/SubName
- **Subscribers:** size
- **Karma gate:** verified threshold or "none published"
- **Promo rules:** brief summary
- **Why fit:** 1 sentence
- **Query patterns (lead-finder):** "phrase 1", "phrase 2"
- **Status:** unlocked / locked-pending-karma / engagement-only / unverified
```

Then verify rules via:

```bash
UA="humla-research/0.1 by u/tremendousquotes"
curl -sL -A "$UA" "https://www.reddit.com/r/SUBNAME/about/rules.json" | python3 -m json.tool | head -100
```

Update the entry with verified data.
