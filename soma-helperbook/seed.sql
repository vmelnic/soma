-- HelperBook Seed Data
-- Run after schema.sql to populate with test data

BEGIN;

-- Users (me + 12 providers)
INSERT INTO users (id, name, phone, role, bio, is_verified, location_lat, location_lon) VALUES
  ('00000000-0000-0000-0000-000000000001', 'Alexandru P.', '+40740555123', 'both', 'Home Repairs, Furniture Assembly', true, 44.4268, 26.1025),
  ('00000000-0000-0000-0000-000000000002', 'Ana M.', '+40721123456', 'provider', 'Hair Stylist, Makeup', true, 44.4350, 26.0980),
  ('00000000-0000-0000-0000-000000000003', 'Ion P.', '+40722234567', 'provider', 'Plumber', true, 44.4180, 26.1100),
  ('00000000-0000-0000-0000-000000000004', 'Elena D.', '+40723345678', 'provider', 'House Cleaning, Laundry', true, 44.4400, 26.0900),
  ('00000000-0000-0000-0000-000000000005', 'Mihai R.', '+40724456789', 'provider', 'Electrician', false, 44.4100, 26.1200),
  ('00000000-0000-0000-0000-000000000006', 'Sofia L.', '+40725567890', 'provider', 'Massage, Physiotherapy', true, 44.4500, 26.0850),
  ('00000000-0000-0000-0000-000000000007', 'Andrei T.', '+40726678901', 'provider', 'Personal Trainer, Nutrition', true, 44.4220, 26.1150),
  ('00000000-0000-0000-0000-000000000008', 'Maria V.', '+40727789012', 'provider', 'Tutoring, Translation', true, 44.4300, 26.0950),
  ('00000000-0000-0000-0000-000000000009', 'Cristian B.', '+40728890123', 'provider', 'Photography', false, 44.4150, 26.1050),
  ('00000000-0000-0000-0000-000000000010', 'Raluca S.', '+40729901234', 'provider', 'Interior Design', true, 44.4380, 26.0920),
  ('00000000-0000-0000-0000-000000000011', 'Dan C.', '+40730012345', 'provider', 'Carpentry, Furniture', true, 44.4200, 26.1080),
  ('00000000-0000-0000-0000-000000000012', 'Ioana F.', '+40731123456', 'provider', 'Pet Sitting, Dog Walking', true, 44.4450, 26.0870),
  ('00000000-0000-0000-0000-000000000013', 'Victor N.', '+40732234567', 'provider', 'Auto Mechanic', true, 44.4120, 26.1180);

-- Provider profiles
INSERT INTO provider_profiles (user_id, bio_extended, service_area_radius, communication_languages) VALUES
  ('00000000-0000-0000-0000-000000000002', 'Professional hair stylist with 8 years of experience. Specialized in coloring and styling.', 10, ARRAY['en', 'ro']),
  ('00000000-0000-0000-0000-000000000003', 'Licensed plumber. Available for emergencies 24/7.', 15, ARRAY['ro']),
  ('00000000-0000-0000-0000-000000000004', 'Deep cleaning specialist. Eco-friendly products only.', 20, ARRAY['ro', 'ru']),
  ('00000000-0000-0000-0000-000000000006', 'Certified massage therapist. Sports and relaxation massage.', 12, ARRAY['en', 'ro']),
  ('00000000-0000-0000-0000-000000000007', 'Certified personal trainer. Custom meal plans included.', 25, ARRAY['en', 'ro']),
  ('00000000-0000-0000-0000-000000000008', 'Tutoring in English, French, and Mathematics. All levels.', 30, ARRAY['en', 'ro', 'fr']);

-- Connections (some accepted, some pending)
INSERT INTO connections (requester_id, recipient_id, status) VALUES
  ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', 'accepted'),
  ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000003', 'accepted'),
  ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000004', 'accepted'),
  ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000006', 'accepted'),
  ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000007', 'pending'),
  ('00000000-0000-0000-0000-000000000008', '00000000-0000-0000-0000-000000000001', 'pending');

-- Chats (direct chats with 4 providers)
INSERT INTO chats (id, type, created_by) VALUES
  ('c0000000-0000-0000-0000-000000000001', 'direct', '00000000-0000-0000-0000-000000000001'),
  ('c0000000-0000-0000-0000-000000000002', 'direct', '00000000-0000-0000-0000-000000000001'),
  ('c0000000-0000-0000-0000-000000000003', 'direct', '00000000-0000-0000-0000-000000000001'),
  ('c0000000-0000-0000-0000-000000000004', 'direct', '00000000-0000-0000-0000-000000000001');

-- Chat members
INSERT INTO chat_members (chat_id, user_id) VALUES
  ('c0000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000001'),
  ('c0000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002'),
  ('c0000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001'),
  ('c0000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000003'),
  ('c0000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000001'),
  ('c0000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000004'),
  ('c0000000-0000-0000-0000-000000000004', '00000000-0000-0000-0000-000000000001'),
  ('c0000000-0000-0000-0000-000000000004', '00000000-0000-0000-0000-000000000006');

-- Messages (realistic conversations)
INSERT INTO messages (chat_id, sender_id, type, content, status) VALUES
  -- Chat with Ana (hair stylist)
  ('c0000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', 'text', 'Hi! I saw your profile. Do you have availability this week?', 'read'),
  ('c0000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000001', 'text', 'Yes! I have slots on Wednesday and Friday afternoon.', 'read'),
  ('c0000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', 'text', 'Wednesday at 3 PM works for me. What services do you offer?', 'read'),
  ('c0000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000001', 'text', 'I do haircuts, coloring, and styling. Full price list on my profile.', 'read'),
  ('c0000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', 'text', 'Perfect, I will book a haircut and styling. See you Wednesday!', 'read'),
  -- Chat with Ion (plumber)
  ('c0000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001', 'text', 'Hi Ion, I have a leaky faucet in the kitchen. Can you help?', 'read'),
  ('c0000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000003', 'text', 'Sure, I can come today between 2-4 PM. Does that work?', 'read'),
  ('c0000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001', 'text', 'That works great. Address is Str. Victoriei 45, ap. 12.', 'read'),
  ('c0000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000003', 'text', 'On my way! Should be there in 30 minutes.', 'read'),
  ('c0000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000003', 'text', 'All done! The pipe is fixed. I also noticed the bathroom faucet needs a new washer — I can do that next time.', 'read'),
  ('c0000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001', 'text', 'Thanks Ion! Great work as always. Will call you about the bathroom.', 'read'),
  -- Chat with Elena (cleaning)
  ('c0000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000001', 'text', 'Hello Elena, do you offer deep cleaning services?', 'read'),
  ('c0000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000004', 'text', 'Yes! I do regular and deep cleaning. For a 2-bedroom apartment, deep cleaning takes about 3 hours.', 'read'),
  ('c0000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000004', 'text', 'I can come on Saturday morning at 9 AM. Would that work?', 'read'),
  ('c0000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000001', 'text', 'Saturday at 9 is perfect. See you then!', 'delivered'),
  -- Chat with Sofia (massage)
  ('c0000000-0000-0000-0000-000000000004', '00000000-0000-0000-0000-000000000006', 'text', 'Hello! Ready for your massage session?', 'read'),
  ('c0000000-0000-0000-0000-000000000004', '00000000-0000-0000-0000-000000000001', 'text', 'Yes, same time as usual please. My back has been really tense.', 'read'),
  ('c0000000-0000-0000-0000-000000000004', '00000000-0000-0000-0000-000000000006', 'text', 'I will focus on the upper back and shoulders. Your session is confirmed for Thursday at 5 PM.', 'read'),
  ('c0000000-0000-0000-0000-000000000004', '00000000-0000-0000-0000-000000000001', 'text', 'Thank you Sofia!', 'sent');

-- Appointments (mix of past, current, and future)
INSERT INTO appointments (id, client_id, provider_id, service, start_time, end_time, status, rate_amount, rate_currency) VALUES
  ('a0000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', 'Hair Styling', '2026-04-09 15:00:00', '2026-04-09 16:00:00', 'confirmed', 150, 'EUR'),
  ('a0000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000006', 'Deep Tissue Massage', '2026-04-10 17:00:00', '2026-04-10 18:30:00', 'confirmed', 200, 'EUR'),
  ('a0000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000004', 'Deep Cleaning', '2026-04-12 09:00:00', '2026-04-12 12:00:00', 'proposed', 400, 'EUR'),
  ('a0000000-0000-0000-0000-000000000004', '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000008', 'English Tutoring', '2026-04-15 10:00:00', '2026-04-15 11:00:00', 'confirmed', 100, 'EUR'),
  ('a0000000-0000-0000-0000-000000000005', '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000007', 'Personal Training', '2026-04-18 14:00:00', '2026-04-18 15:00:00', 'proposed', 120, 'EUR'),
  ('a0000000-0000-0000-0000-000000000006', '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', 'Haircut', '2026-04-05 11:00:00', '2026-04-05 11:45:00', 'completed', 80, 'EUR'),
  ('a0000000-0000-0000-0000-000000000007', '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000003', 'Plumbing Repair', '2026-04-02 16:00:00', '2026-04-02 17:00:00', 'completed', 250, 'EUR');

-- Reviews for completed appointments
INSERT INTO reviews (appointment_id, reviewer_id, reviewed_id, rating, feedback) VALUES
  ('a0000000-0000-0000-0000-000000000006', '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', 5, 'Excellent haircut, very professional. Ana really knows her craft!'),
  ('a0000000-0000-0000-0000-000000000006', '00000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001', 5, 'Great client, always on time and friendly.'),
  ('a0000000-0000-0000-0000-000000000007', '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000003', 4, 'Fixed the leak quickly. Very knowledgeable. Will use again.'),
  ('a0000000-0000-0000-0000-000000000007', '00000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000001', 5, 'Pleasant to work with. Clear communication about the problem.');

COMMIT;
