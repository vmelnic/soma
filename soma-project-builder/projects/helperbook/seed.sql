-- HelperBook Seed Data
BEGIN;

INSERT INTO users (id, name, phone, email, role, bio, is_verified, location_lat, location_lon) VALUES
  ('00000000-0000-0000-0000-000000000001', 'Alexandru P.', '+40740555123', 'alexandru@example.com', 'both', 'Home Repairs, Furniture Assembly', true, 44.4268, 26.1025),
  ('00000000-0000-0000-0000-000000000002', 'Ana M.', '+40721123456', 'ana@example.com', 'provider', 'Hair Stylist, Makeup', true, 44.4350, 26.0980),
  ('00000000-0000-0000-0000-000000000003', 'Ion P.', '+40722234567', 'ion@example.com', 'provider', 'Plumber', true, 44.4180, 26.1100),
  ('00000000-0000-0000-0000-000000000004', 'Elena D.', '+40723345678', 'elena@example.com', 'provider', 'House Cleaning, Laundry', true, 44.4400, 26.0900),
  ('00000000-0000-0000-0000-000000000005', 'Mihai R.', '+40724456789', 'mihai@example.com', 'provider', 'Electrician', false, 44.4100, 26.1200),
  ('00000000-0000-0000-0000-000000000006', 'Sofia L.', '+40725567890', 'sofia@example.com', 'provider', 'Massage, Physiotherapy', true, 44.4500, 26.0850),
  ('00000000-0000-0000-0000-000000000007', 'Andrei T.', '+40726678901', 'andrei@example.com', 'provider', 'Personal Trainer, Nutrition', true, 44.4220, 26.1150),
  ('00000000-0000-0000-0000-000000000008', 'Maria V.', '+40727789012', 'maria@example.com', 'provider', 'Tutoring, Translation', true, 44.4300, 26.0950);

INSERT INTO provider_profiles (user_id, bio_extended, service_area_radius, communication_languages) VALUES
  ('00000000-0000-0000-0000-000000000002', 'Professional hair stylist with 8 years of experience.', 10, ARRAY['en', 'ro']),
  ('00000000-0000-0000-0000-000000000003', 'Licensed plumber. Available for emergencies 24/7.', 15, ARRAY['ro']),
  ('00000000-0000-0000-0000-000000000004', 'Deep cleaning specialist. Eco-friendly products only.', 20, ARRAY['ro', 'ru']),
  ('00000000-0000-0000-0000-000000000006', 'Certified massage therapist. Sports and relaxation massage.', 12, ARRAY['en', 'ro']),
  ('00000000-0000-0000-0000-000000000007', 'Certified personal trainer. Custom meal plans included.', 25, ARRAY['en', 'ro']),
  ('00000000-0000-0000-0000-000000000008', 'Tutoring in English, French, and Mathematics.', 30, ARRAY['en', 'ro', 'fr']);

INSERT INTO connections (requester_id, recipient_id, status) VALUES
  ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', 'accepted'),
  ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000003', 'accepted'),
  ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000004', 'accepted'),
  ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000006', 'accepted'),
  ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000007', 'pending'),
  ('00000000-0000-0000-0000-000000000008', '00000000-0000-0000-0000-000000000001', 'pending');

INSERT INTO chats (id, type, created_by) VALUES
  ('c0000000-0000-0000-0000-000000000001', 'direct', '00000000-0000-0000-0000-000000000001'),
  ('c0000000-0000-0000-0000-000000000002', 'direct', '00000000-0000-0000-0000-000000000001');

INSERT INTO chat_members (chat_id, user_id) VALUES
  ('c0000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000001'),
  ('c0000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002'),
  ('c0000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001'),
  ('c0000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000003');

INSERT INTO messages (chat_id, sender_id, type, content, status) VALUES
  ('c0000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', 'text', 'Hi! Do you have availability this week?', 'read'),
  ('c0000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000001', 'text', 'Yes, Wednesday and Friday afternoon work.', 'read'),
  ('c0000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001', 'text', 'Hi Ion, leaky faucet in the kitchen. Can you help?', 'read'),
  ('c0000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000003', 'text', 'Sure, I can come today between 2-4 PM.', 'read');

INSERT INTO appointments (id, client_id, provider_id, service, start_time, end_time, status, rate_amount) VALUES
  ('a0000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', 'Hair Styling', '2026-05-01 15:00', '2026-05-01 16:00', 'confirmed', 150),
  ('a0000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000003', 'Plumbing Repair', '2026-04-20 16:00', '2026-04-20 17:00', 'completed', 250),
  ('a0000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000006', 'Deep Tissue Massage', '2026-05-05 17:00', '2026-05-05 18:30', 'proposed', 200);

INSERT INTO reviews (appointment_id, reviewer_id, reviewed_id, rating, feedback) VALUES
  ('a0000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000003', 4, 'Fixed the leak quickly. Will use again.');

COMMIT;
