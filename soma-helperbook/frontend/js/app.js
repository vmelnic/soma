// Main App Logic — Routing and State

const routes = {
  contacts: renderContacts,
  chat: renderChat,
  calendar: renderCalendar,
  profile: renderProfile
};

let currentView = 'contacts';

function navigate(view, params = {}) {
  currentView = view;

  // Restore header and nav visibility (chat view hides them)
  const header = document.getElementById('header');
  const bottomNav = document.getElementById('bottom-nav');
  if (view !== 'chat' || params.list) {
    header.style.display = '';
    bottomNav.style.display = '';
    document.getElementById('main').style.paddingBottom = '';
  }

  // Clear main content
  const main = document.getElementById('main');
  main.innerHTML = '';

  // Render view
  if (routes[view]) {
    routes[view](params);
  }

  // Update nav (use base view for nav highlighting)
  const navView = view;
  renderNav(navView);
}

// SOMA status polling
async function checkSomaStatus() {
  try {
    const status = await api.checkStatus();
    const dot = document.getElementById('soma-dot');
    const label = document.getElementById('soma-label');
    if (status.soma) {
      dot.className = 'w-2 h-2 rounded-full bg-green-400';
      label.className = 'text-xs text-green-600 font-medium';
      label.textContent = 'SOMA';
    } else {
      dot.className = 'w-2 h-2 rounded-full bg-gray-300';
      label.className = 'text-xs text-gray-400';
      label.textContent = 'SOMA';
    }
  } catch (e) {
    const dot = document.getElementById('soma-dot');
    const label = document.getElementById('soma-label');
    dot.className = 'w-2 h-2 rounded-full bg-gray-300';
    label.className = 'text-xs text-gray-400';
    label.textContent = 'SOMA';
  }
}

// Seed test data if the database is empty
async function seedTestData() {
  try {
    const countResult = await api.count({ table: 'users' });
    const rows = SomaAPI.extractRows(countResult);
    // Count returns an Int value — check if it's 0
    const count = (typeof rows === 'number') ? rows : 0;
    if (count > 0) {
      console.log('[seed] Database has ' + count + ' users, skipping seed');
      // Load current user ID
      try {
        const meResult = await api.query("SELECT id FROM users WHERE phone = '+40740555123' LIMIT 1");
        const meRows = SomaAPI.extractRows(meResult);
        if (meRows && meRows.length > 0) {
          window.SOMA_USER_ID = meRows[0].id;
          console.log('[seed] My user ID:', window.SOMA_USER_ID);
        }
      } catch(e) { console.log('[seed] Could not load user ID'); }
      return;
    }

    console.log('[seed] Database empty, seeding test data...');

    // Insert "me" user
    await api.execute(
      "INSERT INTO users (name, phone, role, bio, is_verified) VALUES " +
      "('Alexandru P.', '+40740555123', 'both', 'Home Repairs, Furniture Assembly', true)"
    );

    // Get my user ID for later use
    const meResult = await api.query("SELECT id FROM users WHERE phone = '+40740555123' LIMIT 1");
    const meRows = SomaAPI.extractRows(meResult);
    if (meRows && meRows.length > 0) {
      window.SOMA_USER_ID = meRows[0].id;
      console.log('[seed] My user ID:', window.SOMA_USER_ID);
    }

    // Insert provider users
    const providers = [
      "('Ana M.', '+40721123456', 'provider', 'Hair Stylist, Makeup', true)",
      "('Ion P.', '+40722234567', 'provider', 'Plumber', true)",
      "('Elena D.', '+40723345678', 'provider', 'House Cleaning, Laundry', true)",
      "('Mihai R.', '+40724456789', 'provider', 'Electrician', false)",
      "('Sofia L.', '+40725567890', 'provider', 'Massage, Physiotherapy', true)",
      "('Andrei T.', '+40726678901', 'provider', 'Personal Trainer, Nutrition', true)",
      "('Maria V.', '+40727789012', 'provider', 'Tutoring, Translation', true)",
      "('Cristian B.', '+40728890123', 'provider', 'Photography', false)",
      "('Raluca S.', '+40729901234', 'provider', 'Interior Design', true)",
      "('Dan C.', '+40730012345', 'provider', 'Carpentry, Furniture', true)",
      "('Ioana F.', '+40731123456', 'provider', 'Pet Sitting, Dog Walking', true)",
      "('Victor N.', '+40732234567', 'provider', 'Auto Mechanic', true)",
    ];
    await api.execute(
      "INSERT INTO users (name, phone, role, bio, is_verified) VALUES " +
      providers.join(", ")
    );

    // Insert provider_profiles for some providers
    await api.execute(
      "INSERT INTO provider_profiles (user_id, bio_extended, service_area_radius) VALUES " +
      "(2, 'Professional hair stylist with 8 years of experience', 10), " +
      "(3, 'Licensed plumber, available for emergencies', 15), " +
      "(4, 'Deep cleaning specialist', 20), " +
      "(6, 'Certified massage therapist', 12)"
    );

    // Insert some chats
    await api.execute(
      "INSERT INTO chats (type, name, created_by) VALUES " +
      "('direct', NULL, 1), " +
      "('direct', NULL, 1), " +
      "('direct', NULL, 1), " +
      "('direct', NULL, 1)"
    );

    // Insert messages (chat_id 1 = with Ana, 2 = with Ion, 3 = with Elena, 4 = with Sofia)
    await api.execute(
      "INSERT INTO messages (chat_id, sender_id, type, content, status) VALUES " +
      "(1, 2, 'text', 'Hi! I saw your profile. Do you have availability this week?', 'read'), " +
      "(1, 1, 'text', 'Yes! I have slots on Wednesday and Friday afternoon.', 'read'), " +
      "(1, 2, 'text', 'Wednesday at 3 PM works for me.', 'read'), " +
      "(1, 1, 'text', 'Perfect. I will book you in.', 'read'), " +
      "(1, 2, 'text', 'See you tomorrow at 3!', 'read'), " +
      "(2, 1, 'text', 'Hi Ion, I have a leaky faucet in the kitchen. Can you help?', 'read'), " +
      "(2, 3, 'text', 'Sure, I can come today between 2-4 PM. Does that work?', 'read'), " +
      "(2, 1, 'text', 'That works. Address is Str. Victoriei 45.', 'read'), " +
      "(2, 3, 'text', 'The pipe is fixed now.', 'read'), " +
      "(3, 1, 'text', 'Hello Elena, do you offer deep cleaning services?', 'read'), " +
      "(3, 4, 'text', 'Yes! I do regular and deep cleaning.', 'read'), " +
      "(3, 4, 'text', 'I can come on Saturday morning.', 'read'), " +
      "(4, 6, 'text', 'Hello! Ready for your massage session?', 'read'), " +
      "(4, 1, 'text', 'Yes, same time as usual please.', 'read'), " +
      "(4, 6, 'text', 'Your next session is confirmed.', 'read')"
    );

    // Insert appointments
    await api.execute(
      "INSERT INTO appointments (client_id, provider_id, service, start_time, end_time, status, rate_amount) VALUES " +
      "(1, 2, 'Hair Styling', '2026-04-09 15:00:00', '2026-04-09 16:00:00', 'confirmed', 150), " +
      "(1, 6, 'Deep Tissue Massage', '2026-04-10 17:00:00', '2026-04-10 18:30:00', 'confirmed', 200), " +
      "(1, 4, 'Deep Cleaning', '2026-04-12 09:00:00', '2026-04-12 12:00:00', 'pending', 400), " +
      "(1, 8, 'English Tutoring', '2026-04-15 10:00:00', '2026-04-15 11:00:00', 'confirmed', 100), " +
      "(1, 7, 'Personal Training', '2026-04-18 14:00:00', '2026-04-18 15:00:00', 'pending', 120), " +
      "(1, 2, 'Haircut', '2026-04-05 11:00:00', '2026-04-05 11:45:00', 'completed', 80), " +
      "(1, 3, 'Plumbing Repair', '2026-04-02 16:00:00', '2026-04-02 17:00:00', 'cancelled', 250)"
    );

    // Insert some reviews
    await api.execute(
      "INSERT INTO reviews (appointment_id, reviewer_id, reviewed_id, rating, feedback) VALUES " +
      "(6, 1, 2, 5, 'Excellent haircut, very professional'), " +
      "(6, 2, 1, 5, 'Great client, always on time')"
    );

    console.log('[seed] Test data seeded successfully');
  } catch (e) {
    console.warn('[seed] Seeding failed (SOMA may not be connected):', e.message);
  }
}

// Initialize app
document.addEventListener('DOMContentLoaded', () => {
  navigate('contacts');
  checkSomaStatus();
  // Seed test data, then refresh contacts view
  seedTestData().then(() => {
    // Re-render current view to pick up seeded data
    if (currentView === 'contacts') {
      navigate('contacts');
    }
  });
  // Poll status every 10 seconds
  setInterval(checkSomaStatus, 10000);
});
