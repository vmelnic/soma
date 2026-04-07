# HelperBook — SOMA Implementation Specification

**Status:** Design  
**Depends on:** SOMA Core, Synaptic Protocol, Plugin System  
**Product Spec:** See HelperBook.md for full product specification

---

## 1. Vision

HelperBook is the first real-world application built entirely with SOMA. No code. No framework. No API design. A network of SOMAs with plugins, communicating via Synaptic Protocol, built by conversing with the Interface SOMA.

HelperBook proves that a complex, multi-feature, multi-platform application — messaging, scheduling, networking, AI, payments, notifications — can exist as SOMA instances with zero traditional code.

---

## 2. Architecture Overview

```
┌──────────────────────────────────────────────────┐
│  User Devices                                     │
│                                                   │
│  ┌──────────────┐  ┌──────────────┐              │
│  │ Interface    │  │ Interface    │  ...          │
│  │ SOMA         │  │ SOMA         │              │
│  │ (browser)    │  │ (iOS/Android)│              │
│  │              │  │              │              │
│  │ Plugins:     │  │ Plugins:     │              │
│  │  dom-render  │  │  uikit-render│              │
│  │  pencil-dsgn │  │  pencil-dsgn │              │
│  │  offline     │  │  offline     │              │
│  │  webrtc      │  │  native-push │              │
│  └──────┬───────┘  └──────┬───────┘              │
│         │                  │                      │
└─────────┼──────────────────┼──────────────────────┘
          │                  │
    Synaptic Protocol  Synaptic Protocol
          │                  │
┌─────────┴──────────────────┴──────────────────────┐
│  Backend SOMA(s)                                   │
│  (one process or many — depends on load)           │
│                                                    │
│  Plugins loaded:                                   │
│    postgres       — all data persistence           │
│    redis          — caching, sessions, pub/sub     │
│    otp-auth       — phone verification, OTP        │
│    social-auth    — Google/Apple Sign-In            │
│    session-mgr    — session tokens, device tracking │
│    messaging      — message storage, delivery      │
│    calendar       — scheduling, reminders           │
│    geolocation    — distance, radius search         │
│    text-search    — semantic service matching       │
│    ai-inference   — smart replies, intent detection │
│    image-proc     — thumbnails, EXIF strip          │
│    smtp           — email notifications             │
│    twilio         — SMS OTP, invite links           │
│    apns + fcm     — push notifications              │
│    id-verify      — face matching for ID Check      │
│    reviews        — rating storage, aggregation     │
│    analytics      — provider dashboards             │
│    localization   — en/ro/ru string management      │
│    s3             — media file storage              │
│    crypto         — token signing, hashing          │
│                                                    │
│  LoRA Knowledge:                                   │
│    Each plugin contributes LoRA weights             │
│    + HelperBook domain LoRA (booking, services,     │
│      provider/client relationships)                 │
│    + Experiential LoRA (grows from real usage)      │
│                                                    │
└────────────────────────────────────────────────────┘
```

### Single vs Cluster

If one Backend SOMA with all plugins handles the load: it stays one process. If not: split by domain — a Messaging SOMA, a Calendar SOMA, a Network SOMA — each with relevant plugins. They communicate via Synaptic Protocol internally. The architecture is identical either way.

---

## 3. The Interface SOMA — Building HelperBook's UI

### 3.1 The Conversational Design Process

The Interface SOMA has a chat-like input. You describe what you want. It renders.

```
You: "I need a messaging app for connecting clients 
      with service providers. Start with the main 
      navigation: Contacts, Chats, Calendar, Profile"

Interface SOMA: [renders bottom tab bar with 4 icons + labels]

You: [import helperbook.pen design file]

Interface SOMA: [absorbs design language — applies HelperBook 
                  colors, typography, spacing to the tab bar]

You: "The Contacts tab has two sub-tabs: 
      Contacts (my connections) and Network (discovery)"

Interface SOMA: [renders segmented control in the Contacts view]

You: "Each contact card shows: photo, name, service tags, 
      rating stars, distance, and online status dot"

Interface SOMA: [renders contact card component]

You: "Connect to backend SOMA and populate with real data"

Interface SOMA: [synaptic connection established]
Backend SOMA: [sends semantic signal with contacts data]
Interface SOMA: [populates cards with real data in the design language]
```

### 3.2 Semantic Signals for HelperBook Views

The Backend SOMA sends semantic data, not HTML. The Interface SOMA renders it.

**Contacts List:**
```json
{
  "view": "contact_list",
  "sub_tab": "contacts",
  "data": [
    {
      "id": "user_abc",
      "name": "Ana M.",
      "photo_url": "soma://media/photos/ana.jpg",
      "role": "provider",
      "services": ["Hair Stylist", "Makeup"],
      "rating": 4.8,
      "review_count": 23,
      "distance_km": 2.3,
      "online": true,
      "badges": ["verified", "id_checked"],
      "favorited": true
    }
  ],
  "folders": ["Home", "Kids", "VIP"],
  "actions": ["search", "filter", "add_contact"],
  "filters_available": ["service", "location", "rating", "availability"]
}
```

**Chat View:**
```json
{
  "view": "chat",
  "peer": {"id": "user_abc", "name": "Ana M.", "online": true, "typing": false},
  "messages": [
    {
      "id": "msg_1",
      "from": "user_abc",
      "type": "text",
      "content": "Can you come Thursday at 3?",
      "timestamp": "2026-04-07T14:30:00Z",
      "status": "read"
    },
    {
      "id": "msg_2",
      "from": "me",
      "type": "text",
      "content": "Sure, what address?",
      "timestamp": "2026-04-07T14:31:00Z",
      "status": "delivered"
    },
    {
      "id": "msg_3",
      "type": "appointment_card",
      "data": {
        "service": "Hair Styling",
        "date": "2026-04-10",
        "time": "15:00",
        "duration": 60,
        "location": "123 Main St",
        "rate": {"amount": 35, "currency": "EUR", "type": "fixed"},
        "status": "proposed"
      },
      "actions": ["confirm", "dismiss", "suggest_change"]
    }
  ],
  "input": {
    "type": "chat_input",
    "ai_suggestions": ["Sounds good", "What time works?", "I'm not available"],
    "quick_replies_available": true,
    "attachment_types": ["photo", "video", "voice", "document", "location", "contact_card"]
  }
}
```

**Calendar View:**
```json
{
  "view": "calendar",
  "month": "2026-04",
  "days_with_events": [7, 10, 12, 15, 20],
  "selected_day": 10,
  "events": [
    {
      "id": "apt_1",
      "time": "15:00-16:00",
      "service": "Hair Styling",
      "with": {"name": "Ana M.", "photo_url": "soma://media/photos/ana.jpg"},
      "status": "confirmed",
      "location": "123 Main St"
    }
  ],
  "actions": ["create_appointment"]
}
```

### 3.3 Real-Time Updates via Synaptic Protocol

The Interface SOMA subscribes to real-time channels:

```
Interface → Backend: SUBSCRIBE {channel: 100, metadata: {topic: "chat:user_abc"}}
Interface → Backend: SUBSCRIBE {channel: 101, metadata: {topic: "presence"}}
Interface → Backend: SUBSCRIBE {channel: 102, metadata: {topic: "notifications"}}

Backend → Interface: STREAM_DATA {channel: 100, payload: {
  type: "new_message",
  message: {from: "user_abc", type: "text", content: "hello!"}
}}

Backend → Interface: STREAM_DATA {channel: 101, payload: {
  type: "status_change",
  user_id: "user_abc",
  online: false
}}

Backend → Interface: STREAM_DATA {channel: 102, payload: {
  type: "appointment_confirmed",
  appointment_id: "apt_1"
}}
```

The Interface SOMA updates its rendered UI in response to each signal. No page reload. No polling. No WebSocket wrapper. Pure Synaptic Protocol.

### 3.4 File Uploads

Profile photo upload:

```
1. User selects photo in Interface SOMA

2. Interface SOMA → Backend SOMA: CHUNK_START {
     channel: 7,
     metadata: {
       filename: "profile.jpg",
       total_size: 2500000,
       content_type: "image/jpeg",
       purpose: "profile_photo"
     }
   }

3. Interface SOMA → Backend SOMA: CHUNK_DATA × N

4. Backend SOMA receives all chunks, reassembles

5. Backend SOMA → [image-proc plugin]: generate thumbnail
   Backend SOMA → [s3 plugin]: store original + thumbnail
   Backend SOMA → [postgres plugin]: update user.photo_url

6. Backend SOMA → Interface SOMA: DATA {
     payload: {
       type: "photo_uploaded",
       thumbnail: [binary signal with thumbnail bytes],
       url: "soma://media/photos/user_xyz_thumb.jpg"
     }
   }

7. Interface SOMA renders the preview immediately from thumbnail data
```

### 3.5 Voice Messages

```
1. User holds record button in Interface SOMA

2. Interface SOMA → Backend SOMA: STREAM_START {
     channel: 20,
     metadata: {type: "voice_message", codec: "opus", chat_id: "chat_abc"}
   }

3. Interface SOMA → Backend SOMA: STREAM_DATA {channel: 20, payload: [opus frames]}
   (streaming as user records)

4. User releases button

5. Interface SOMA → Backend SOMA: STREAM_END {channel: 20}

6. Backend SOMA → [audio-proc plugin]: normalize, measure duration
   Backend SOMA → [s3 plugin]: store
   Backend SOMA → [messaging plugin]: create message record
   Backend SOMA → recipient Interface SOMA: DATA {new voice message}
```

---

## 4. Backend SOMA — HelperBook Domain Logic

### 4.1 How the Backend Learns HelperBook

The Backend SOMA starts with plugin knowledge (PostgreSQL, Redis, Auth, etc.). The HelperBook-specific domain logic is provided through:

1. **Domain LoRA plugin** — pre-trained on HelperBook's business rules:
   - Dual-role user model (client + provider)
   - Connection request lifecycle
   - Appointment lifecycle (proposed → confirmed → completed)
   - Service completion + review flow
   - Rate limiting rules
   - Badge/verification logic

2. **Schema knowledge** — the PostgreSQL plugin LoRA includes training on HelperBook's database schema

3. **Conversational bootstrapping** — like the Interface SOMA, you can talk to the Backend SOMA:

```
"Create a table for users with: id, phone, name, photo_url, 
 location_lat, location_lon, bio, role (client/provider/both), 
 is_verified, is_id_checked, created_at"

Backend SOMA: [generates and executes schema creation via PostgreSQL plugin]

"Create a table for connections: id, requester_id, recipient_id, 
 status (pending/accepted/declined/blocked), message, created_at"

Backend SOMA: [creates table]

"When a user sends a connection request, check if they've 
 exceeded 5 requests today (unless they're Plus subscribers)"

Backend SOMA: [stores this as a business rule in its experiential memory, 
               creates the query pattern, associates it with the 
               connection_request convention]
```

### 4.2 Database Schema

The schema is created conversationally, not in SQL migration files. The Backend SOMA's experiential memory encodes the schema knowledge. Key tables:

**users** — id, phone, name, photo_url, bio, location, role, subscription_plan, verified, id_checked, slug, locale, currency, created_at

**connections** — id, requester_id, recipient_id, status, message, created_at

**messages** — id, chat_id, sender_id, type (text/photo/video/voice/document/location/contact_card/appointment_card/service_card), content, media_url, status (sent/delivered/read), reply_to_id, edited_at, deleted_at, created_at

**chats** — id, type (direct/group), name, photo_url, created_by, created_at

**chat_members** — chat_id, user_id, role (member/admin), joined_at, muted_until

**appointments** — id, chat_id, creator_id, client_id, provider_id, service, start_time, end_time, location, rate_amount, rate_currency, rate_type, status (proposed/confirmed/in_progress/completed/dismissed/cancelled/no_show), notes, created_at

**reviews** — id, appointment_id, reviewer_id, reviewed_id, rating, feedback, tags, photos, response, created_at

**services_history** — id, appointment_id, services, hours, rate, total_amount, confirmed_by_client, confirmed_by_provider, disputed, created_at

**provider_profiles** — user_id, bio_extended, certifications, working_schedule, gallery, service_area_radius, communication_languages, response_rate, avg_response_time

**user_services** — user_id, service_id, rate_amount, rate_currency, rate_type (hourly/fixed/negotiable)

**service_categories** — id, parent_id, name_en, name_ro, name_ru, icon

**notifications** — id, user_id, type, title, body, data, read, created_at

**devices** — id, user_id, device_type, push_token, last_active, created_at

**user_settings** — user_id, key, value

**blocked_users** — blocker_id, blocked_id, created_at

**contact_notes** — user_id, contact_id, note_text, updated_at

**contact_folders** — id, user_id, name, position

**contact_folder_members** — folder_id, contact_id

### 4.3 AI Features via Plugins

**Smart Replies (ai-inference plugin):**
```
1. New message arrives
2. Backend SOMA → [ai-inference plugin]: generate_suggestions(conversation_context)
3. Suggestions returned
4. Backend SOMA → Interface SOMA: DATA {
     payload: {type: "ai_suggestions", suggestions: ["Sounds good", "What time?"]}
   }
5. Interface SOMA renders suggestion chips above input
```

**Intent Detection in Chat (ai-inference plugin):**
```
1. Message contains "can you come Thursday at 3?"
2. Backend SOMA → [ai-inference plugin]: detect_intent(message_text)
3. Intent: "schedule_appointment", entities: {day: "Thursday", time: "15:00"}
4. Backend SOMA → Interface SOMA: DATA {
     payload: {type: "appointment_suggestion", prefilled: {day: "Thursday", time: "15:00"}}
   }
5. Interface SOMA renders "Create Appointment" card in chat
```

**AI Search (ai-inference + geolocation + text-search plugins):**
```
User: "I need a babysitter for Saturday evening near downtown"

1. Backend SOMA → [ai-inference plugin]: parse_search(query)
   Result: {service: "Babysitter", time: "Saturday evening", location: "downtown"}

2. Backend SOMA → [geolocation plugin]: geocode("downtown") → lat, lon

3. Backend SOMA → [postgres plugin]: 
   query providers WHERE service="Babysitter" 
   AND available_saturday_evening = true
   AND ST_DWithin(location, point, radius)
   ORDER BY rating DESC, distance ASC

4. Backend SOMA → Interface SOMA: DATA {
     payload: {view: "search_results", results: [...], clarifying_questions: [...]}
   }
```

**Semantic Service Matching (text-search plugin):**
```
User types: "clean my house"

1. Backend SOMA → [text-search plugin]: semantic_match("clean my house", canonical_services)
2. Result: [{service: "House Cleaning", score: 0.95}, {service: "Deep Cleaning", score: 0.82}]
3. Backend SOMA → Interface SOMA: DATA {
     payload: {type: "autocomplete", suggestions: ["House Cleaning", "Deep Cleaning"]}
   }
```

---

## 5. Offline Behavior

The Interface SOMA's offline-cache plugin stores:
- Contact list (last synced)
- Recent chat messages (last N per chat)
- Calendar events
- User profile and settings
- Service history

When offline:
- Interface SOMA renders from cached data
- New messages/actions are queued in local storage
- On reconnect: Synaptic Protocol resumes, queued signals are sent, cache is synced

The Backend SOMA doesn't know about offline behavior. It just receives signals whenever they arrive. The offline-cache plugin is entirely an Interface SOMA concern.

---

## 6. Multi-Device Sync

Multiple Interface SOMAs (phone + browser + tablet) connect to the same Backend SOMA. Each subscribes to the same real-time channels. All receive the same signals.

When Interface SOMA on phone marks a message as read:
```
Phone Interface → Backend: DATA {type: "mark_read", message_ids: [...]}
Backend: updates database
Backend → ALL connected Interface SOMAs: STREAM_DATA {type: "messages_read", ids: [...]}
Browser Interface: updates UI (marks as read)
Tablet Interface: updates UI (marks as read)
```

---

## 7. Push Notifications

When an Interface SOMA is not connected (app closed):

```
1. New message arrives at Backend SOMA
2. Backend SOMA checks: is recipient's Interface SOMA connected? 
   - Yes → send via Synaptic Protocol (real-time)
   - No → Backend SOMA → [apns/fcm plugin]: send push notification
   - Also No and email linked → Backend SOMA → [smtp plugin]: send email (for critical notifications)
```

The notification plugins (APNS, FCM, Twilio, SMTP) are just plugins. The Mind decides when to use them based on connection state and notification rules.

---

## 8. Security for HelperBook

### 8.1 Authentication Flow

```
1. User opens app → Interface SOMA renders login screen
2. User enters phone number
3. Interface SOMA → Backend SOMA: INTENT {payload: "authenticate +1234567890"}
4. Backend SOMA → [otp-auth plugin]: generate OTP, store with expiry
5. Backend SOMA → [twilio plugin]: send SMS with OTP
6. User enters OTP
7. Interface SOMA → Backend SOMA: DATA {type: "verify_otp", phone: "...", otp: "123456"}
8. Backend SOMA → [otp-auth plugin]: verify
9. Backend SOMA → [session-mgr plugin]: create session token
10. Backend SOMA → Interface SOMA: DATA {type: "auth_success", token: "...", user: {...}}
11. Interface SOMA stores token, establishes authenticated Synaptic connection
```

### 8.2 Encrypted Synaptic Connections

All Synaptic Protocol connections are encrypted:
- Key exchange during HANDSHAKE (X25519)
- All subsequent signals encrypted (ChaCha20-Poly1305)
- Session token included in HANDSHAKE for authentication
- Connection rejected if token is invalid or expired

### 8.3 Rate Limiting

The Backend SOMA enforces rate limits from its experiential memory:

```
Mind receives: "send connection request to user_xyz"
Mind checks: how many requests has this user sent today?
  → [postgres plugin]: COUNT(*) FROM connections WHERE requester_id=... AND created_at > today
  → if >= 5 AND subscription != 'plus': respond with rate_limit signal
  → else: proceed with connection request
```

The rate limiting logic lives in the Mind's weights (experiential memory), not in middleware code.

---

## 9. Monetization

### 9.1 Subscription Check

```
User attempts action that requires Plus:
1. Mind checks user's subscription via [postgres plugin]
2. If Plus: proceed
3. If Free and at limit: 
   Backend → Interface: DATA {
     type: "upgrade_prompt",
     reason: "daily_connection_limit",
     message: "You've reached 5 connection requests today. Upgrade for unlimited."
   }
```

### 9.2 Payment Processing

HelperBook doesn't process payments for services (off-platform). But subscription payments:
- Interface SOMA opens payment flow (Apple IAP / Google Play Billing / Stripe web)
- Payment confirmation signal → Backend SOMA updates subscription status
- Stripe plugin handles web payment webhooks

---

## 10. Building HelperBook — The Conversation

This is how HelperBook comes to life. Not a sprint board. Not a Jira ticket. A conversation.

```
Phase 1: "I need a messaging app for service providers and clients..."
  → Basic navigation, contact cards, chat UI, design language

Phase 2: "Users authenticate with phone number + OTP..."
  → Auth flow, session management, profile creation

Phase 3: "Contacts can send connection requests..."
  → Connection lifecycle, network discovery, search

Phase 4: "Chats support text, photos, voice messages..."
  → Messaging, media upload, real-time delivery

Phase 5: "Add appointment cards in chat..."
  → Calendar, scheduling, appointment lifecycle

Phase 6: "After completing a service, both parties review..."
  → Reviews, ratings, service history

Phase 7: "AI suggests replies and detects scheduling intent..."
  → AI features, smart replies, intent detection

Phase 8: "Push notifications when app is closed..."
  → APNS, FCM, email fallback

Phase 9: "The app works offline and syncs when back online..."
  → Offline cache, queue, sync

Phase 10: "Multiple devices stay in sync..."
  → Multi-device, session management

Each phase is a conversation with the SOMA. Each conversation 
changes the SOMA's experiential memory. The application grows 
as the SOMA grows. There is no "codebase" to maintain. 
There is only the SOMA.
```

---

## 11. What Success Looks Like

HelperBook exists. Users message, book, review, discover providers. The application is a network of SOMAs with plugins. Nobody wrote code. The Interface SOMA renders adaptive UI from semantic signals. The Backend SOMA orchestrates data, auth, messaging, scheduling, and AI through plugins.

When a new feature is needed — "add group chat" — someone describes it to the SOMA. The SOMA extends itself. No developer. No sprint. No deploy. Just intent → the feature exists.

When a new platform is needed — "make it work on Apple Watch" — a new Interface SOMA is synthesized with a WatchKit renderer plugin. The Backend SOMA doesn't change. The semantic signals are the same. The watch renders them in its own way.

HelperBook is not just an app. It's proof that SOMA replaces software development for real-world products.
