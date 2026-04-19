-- =============================================================================
-- HelperBook Database Schema
-- =============================================================================
-- PostgreSQL 16 schema for the HelperBook application.
-- Implements all 19 tables from the HelperBook spec (04_HELPERBOOK.md Section 4.2).
--
-- Tables are ordered by dependency so this file can be executed as a single
-- transaction against a fresh database:
--
--   psql -d helperbook -f schema.sql
--
-- =============================================================================

BEGIN;

-- ---------------------------------------------------------------------------
-- 1. users
-- ---------------------------------------------------------------------------
CREATE TABLE users (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    phone            VARCHAR(20) UNIQUE NOT NULL,
    name             TEXT NOT NULL,
    photo_url        TEXT,
    bio              TEXT,
    location_lat     DOUBLE PRECISION,
    location_lon     DOUBLE PRECISION,
    role             VARCHAR(10) DEFAULT 'client'
                     CHECK (role IN ('client', 'provider', 'both')),
    subscription_plan VARCHAR(20) DEFAULT 'free',
    is_verified      BOOLEAN DEFAULT FALSE,
    is_id_checked    BOOLEAN DEFAULT FALSE,
    slug             VARCHAR(100) UNIQUE,
    locale           VARCHAR(5) DEFAULT 'en',
    currency         VARCHAR(3) DEFAULT 'EUR',
    created_at       TIMESTAMP DEFAULT NOW()
);

-- ---------------------------------------------------------------------------
-- 2. connections
-- ---------------------------------------------------------------------------
CREATE TABLE connections (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    requester_id  UUID REFERENCES users(id),
    recipient_id  UUID REFERENCES users(id),
    status        VARCHAR(20) DEFAULT 'pending'
                  CHECK (status IN ('pending', 'accepted', 'declined', 'blocked')),
    message       TEXT,
    created_at    TIMESTAMP DEFAULT NOW()
);

-- ---------------------------------------------------------------------------
-- 3. chats
-- ---------------------------------------------------------------------------
CREATE TABLE chats (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    type        VARCHAR(10) DEFAULT 'direct'
                CHECK (type IN ('direct', 'group')),
    name        TEXT,
    photo_url   TEXT,
    created_by  UUID REFERENCES users(id),
    created_at  TIMESTAMP DEFAULT NOW()
);

-- ---------------------------------------------------------------------------
-- 4. chat_members
-- ---------------------------------------------------------------------------
CREATE TABLE chat_members (
    chat_id     UUID REFERENCES chats(id),
    user_id     UUID REFERENCES users(id),
    role        VARCHAR(10) DEFAULT 'member',
    joined_at   TIMESTAMP DEFAULT NOW(),
    muted_until TIMESTAMP,
    PRIMARY KEY (chat_id, user_id)
);

-- ---------------------------------------------------------------------------
-- 5. messages
-- ---------------------------------------------------------------------------
CREATE TABLE messages (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    chat_id     UUID REFERENCES chats(id),
    sender_id   UUID REFERENCES users(id),
    type        VARCHAR(20) DEFAULT 'text'
                CHECK (type IN ('text', 'photo', 'video', 'voice', 'document',
                                'location', 'contact_card', 'appointment_card',
                                'service_card')),
    content     TEXT,
    media_url   TEXT,
    status      VARCHAR(10) DEFAULT 'sent'
                CHECK (status IN ('sent', 'delivered', 'read')),
    reply_to_id UUID REFERENCES messages(id),
    edited_at   TIMESTAMP,
    deleted_at  TIMESTAMP,
    created_at  TIMESTAMP DEFAULT NOW()
);

-- ---------------------------------------------------------------------------
-- 6. appointments
-- ---------------------------------------------------------------------------
CREATE TABLE appointments (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    chat_id       UUID REFERENCES chats(id),
    creator_id    UUID REFERENCES users(id),
    client_id     UUID REFERENCES users(id),
    provider_id   UUID REFERENCES users(id),
    service       TEXT NOT NULL,
    start_time    TIMESTAMP NOT NULL,
    end_time      TIMESTAMP,
    location      TEXT,
    rate_amount   DECIMAL(10,2),
    rate_currency VARCHAR(3) DEFAULT 'EUR',
    rate_type     VARCHAR(10) DEFAULT 'hourly'
                  CHECK (rate_type IN ('hourly', 'fixed', 'negotiable')),
    status        VARCHAR(20) DEFAULT 'proposed'
                  CHECK (status IN ('proposed', 'confirmed', 'in_progress',
                                    'completed', 'dismissed', 'cancelled',
                                    'no_show')),
    notes         TEXT,
    created_at    TIMESTAMP DEFAULT NOW()
);

-- ---------------------------------------------------------------------------
-- 7. reviews
-- ---------------------------------------------------------------------------
CREATE TABLE reviews (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    appointment_id  UUID REFERENCES appointments(id),
    reviewer_id     UUID REFERENCES users(id),
    reviewed_id     UUID REFERENCES users(id),
    rating          INT CHECK (rating BETWEEN 1 AND 5),
    feedback        TEXT,
    tags            TEXT[],
    photos          TEXT[],
    response        TEXT,
    created_at      TIMESTAMP DEFAULT NOW()
);

-- ---------------------------------------------------------------------------
-- 8. services_history
-- ---------------------------------------------------------------------------
CREATE TABLE services_history (
    id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    appointment_id        UUID REFERENCES appointments(id),
    services              TEXT[],
    hours                 DECIMAL(4,1),
    rate                  DECIMAL(10,2),
    total_amount          DECIMAL(10,2),
    confirmed_by_client   BOOLEAN DEFAULT FALSE,
    confirmed_by_provider BOOLEAN DEFAULT FALSE,
    disputed              BOOLEAN DEFAULT FALSE,
    created_at            TIMESTAMP DEFAULT NOW()
);

-- ---------------------------------------------------------------------------
-- 9. provider_profiles
-- ---------------------------------------------------------------------------
CREATE TABLE provider_profiles (
    user_id                 UUID PRIMARY KEY REFERENCES users(id),
    bio_extended            TEXT,
    certifications          TEXT[],
    working_schedule        JSONB,
    gallery                 TEXT[],
    service_area_radius     INT DEFAULT 25,
    communication_languages TEXT[],
    response_rate           DECIMAL(3,2) DEFAULT 0,
    avg_response_time       INT DEFAULT 0
);

-- ---------------------------------------------------------------------------
-- 10. service_categories
-- ---------------------------------------------------------------------------
CREATE TABLE service_categories (
    id        UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    parent_id UUID REFERENCES service_categories(id),
    name_en   TEXT NOT NULL,
    name_ro   TEXT,
    name_ru   TEXT,
    icon      TEXT
);

-- ---------------------------------------------------------------------------
-- 11. user_services
-- ---------------------------------------------------------------------------
CREATE TABLE user_services (
    user_id       UUID REFERENCES users(id),
    service_id    UUID REFERENCES service_categories(id),
    rate_amount   DECIMAL(10,2),
    rate_currency VARCHAR(3) DEFAULT 'EUR',
    rate_type     VARCHAR(10) DEFAULT 'hourly',
    PRIMARY KEY (user_id, service_id)
);

-- ---------------------------------------------------------------------------
-- 12. notifications
-- ---------------------------------------------------------------------------
CREATE TABLE notifications (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    UUID REFERENCES users(id),
    type       VARCHAR(50) NOT NULL,
    title      TEXT,
    body       TEXT,
    data       JSONB,
    read       BOOLEAN DEFAULT FALSE,
    created_at TIMESTAMP DEFAULT NOW()
);

-- ---------------------------------------------------------------------------
-- 13. devices
-- ---------------------------------------------------------------------------
CREATE TABLE devices (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID REFERENCES users(id),
    device_type VARCHAR(20),
    push_token  TEXT,
    last_active TIMESTAMP DEFAULT NOW(),
    created_at  TIMESTAMP DEFAULT NOW()
);

-- ---------------------------------------------------------------------------
-- 14. user_settings
-- ---------------------------------------------------------------------------
CREATE TABLE user_settings (
    user_id UUID REFERENCES users(id),
    key     VARCHAR(100),
    value   TEXT,
    PRIMARY KEY (user_id, key)
);

-- ---------------------------------------------------------------------------
-- 15. blocked_users
-- ---------------------------------------------------------------------------
CREATE TABLE blocked_users (
    blocker_id UUID REFERENCES users(id),
    blocked_id UUID REFERENCES users(id),
    created_at TIMESTAMP DEFAULT NOW(),
    PRIMARY KEY (blocker_id, blocked_id)
);

-- ---------------------------------------------------------------------------
-- 16. contact_notes
-- ---------------------------------------------------------------------------
CREATE TABLE contact_notes (
    user_id    UUID REFERENCES users(id),
    contact_id UUID REFERENCES users(id),
    note_text  TEXT,
    updated_at TIMESTAMP DEFAULT NOW(),
    PRIMARY KEY (user_id, contact_id)
);

-- ---------------------------------------------------------------------------
-- 17. contact_folders
-- ---------------------------------------------------------------------------
CREATE TABLE contact_folders (
    id       UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id  UUID REFERENCES users(id),
    name     TEXT NOT NULL,
    position INT DEFAULT 0
);

-- ---------------------------------------------------------------------------
-- 18. contact_folder_members
-- ---------------------------------------------------------------------------
CREATE TABLE contact_folder_members (
    folder_id  UUID REFERENCES contact_folders(id),
    contact_id UUID REFERENCES users(id),
    PRIMARY KEY (folder_id, contact_id)
);

-- ---------------------------------------------------------------------------
-- 19. _soma_migrations
-- ---------------------------------------------------------------------------
CREATE TABLE _soma_migrations (
    id           SERIAL PRIMARY KEY,
    description  TEXT NOT NULL,
    sql_executed TEXT NOT NULL,
    executed_at  TIMESTAMP DEFAULT NOW(),
    soma_id      TEXT NOT NULL,
    reversible   BOOLEAN DEFAULT TRUE,
    rollback_sql TEXT
);


-- =============================================================================
-- Indexes
-- =============================================================================

-- users
CREATE INDEX idx_users_phone ON users (phone);
CREATE INDEX idx_users_slug ON users (slug);
CREATE INDEX idx_users_location ON users (location_lat, location_lon);

-- connections
CREATE INDEX idx_connections_requester_status ON connections (requester_id, status);
CREATE INDEX idx_connections_recipient_status ON connections (recipient_id, status);

-- messages
CREATE INDEX idx_messages_chat_created ON messages (chat_id, created_at);
CREATE INDEX idx_messages_sender ON messages (sender_id);

-- appointments
CREATE INDEX idx_appointments_client_status ON appointments (client_id, status);
CREATE INDEX idx_appointments_provider_status ON appointments (provider_id, status);
CREATE INDEX idx_appointments_start_time ON appointments (start_time);

-- reviews
CREATE INDEX idx_reviews_reviewed ON reviews (reviewed_id);
CREATE INDEX idx_reviews_appointment ON reviews (appointment_id);

-- notifications
CREATE INDEX idx_notifications_user_read ON notifications (user_id, read);


-- =============================================================================
-- Seed Data: Service Categories
-- =============================================================================

-- Top-level categories
INSERT INTO service_categories (id, parent_id, name_en) VALUES
    ('a0000000-0000-0000-0000-000000000001', NULL, 'Hair & Beauty'),
    ('a0000000-0000-0000-0000-000000000002', NULL, 'Home Services'),
    ('a0000000-0000-0000-0000-000000000003', NULL, 'Education'),
    ('a0000000-0000-0000-0000-000000000004', NULL, 'Health & Wellness');

-- Hair & Beauty subcategories
INSERT INTO service_categories (id, parent_id, name_en) VALUES
    ('b0000000-0000-0000-0000-000000000001', 'a0000000-0000-0000-0000-000000000001', 'Hair Styling'),
    ('b0000000-0000-0000-0000-000000000002', 'a0000000-0000-0000-0000-000000000001', 'Makeup');

-- Home Services subcategories
INSERT INTO service_categories (id, parent_id, name_en) VALUES
    ('b0000000-0000-0000-0000-000000000003', 'a0000000-0000-0000-0000-000000000002', 'House Cleaning'),
    ('b0000000-0000-0000-0000-000000000004', 'a0000000-0000-0000-0000-000000000002', 'Plumbing');

-- Education subcategories
INSERT INTO service_categories (id, parent_id, name_en) VALUES
    ('b0000000-0000-0000-0000-000000000005', 'a0000000-0000-0000-0000-000000000003', 'Tutoring'),
    ('b0000000-0000-0000-0000-000000000006', 'a0000000-0000-0000-0000-000000000003', 'Language Lessons');

-- Health & Wellness subcategories
INSERT INTO service_categories (id, parent_id, name_en) VALUES
    ('b0000000-0000-0000-0000-000000000007', 'a0000000-0000-0000-0000-000000000004', 'Massage'),
    ('b0000000-0000-0000-0000-000000000008', 'a0000000-0000-0000-0000-000000000004', 'Personal Training');


-- =============================================================================
-- Record this migration
-- =============================================================================
INSERT INTO _soma_migrations (description, sql_executed, soma_id, reversible, rollback_sql)
VALUES (
    'Initial HelperBook schema: 19 tables, indexes, seed categories',
    'schema.sql',
    'helperbook-schema-v1',
    TRUE,
    'DROP TABLE IF EXISTS contact_folder_members, contact_folders, contact_notes, blocked_users, user_settings, devices, notifications, user_services, service_categories, provider_profiles, services_history, reviews, appointments, messages, chat_members, chats, connections, users, _soma_migrations CASCADE;'
);

COMMIT;
