// Calendar View

const MOCK_APPOINTMENTS = [
  {
    id: 'a1', date: '2026-04-09', time: '15:00', duration: 60,
    service: 'Hair Styling', provider: 'Ana M.', providerId: '1',
    status: 'confirmed', notes: 'Regular appointment'
  },
  {
    id: 'a2', date: '2026-04-10', time: '17:00', duration: 90,
    service: 'Deep Tissue Massage', provider: 'Sofia L.', providerId: '5',
    status: 'confirmed', notes: ''
  },
  {
    id: 'a3', date: '2026-04-12', time: '09:00', duration: 180,
    service: 'Deep Cleaning', provider: 'Elena D.', providerId: '3',
    status: 'pending', notes: '2-bedroom apartment'
  },
  {
    id: 'a4', date: '2026-04-15', time: '10:00', duration: 60,
    service: 'English Tutoring', provider: 'Maria V.', providerId: '7',
    status: 'confirmed', notes: 'Advanced conversation'
  },
  {
    id: 'a5', date: '2026-04-18', time: '14:00', duration: 60,
    service: 'Personal Training', provider: 'Andrei T.', providerId: '6',
    status: 'pending', notes: 'First session'
  },
  {
    id: 'a6', date: '2026-04-05', time: '11:00', duration: 45,
    service: 'Haircut', provider: 'Ana M.', providerId: '1',
    status: 'confirmed', notes: 'Completed'
  },
  {
    id: 'a7', date: '2026-04-02', time: '16:00', duration: 60,
    service: 'Plumbing Repair', provider: 'Ion P.', providerId: '2',
    status: 'cancelled', notes: 'Rescheduled'
  }
];

function renderCalendar(params = {}) {
  const main = document.getElementById('main');
  const today = new Date();
  const selectedDate = params.selectedDate || today.toISOString().split('T')[0];
  const viewMonth = params.month !== undefined ? params.month : today.getMonth();
  const viewYear = params.year || today.getFullYear();

  const container = document.createElement('div');
  container.className = 'view-enter';

  // Month header
  const monthHeader = document.createElement('div');
  monthHeader.className = 'px-4 pt-4 pb-2 flex items-center justify-between';

  const prevBtn = document.createElement('button');
  prevBtn.className = 'p-2 text-gray-400 hover:text-gray-600';
  const prevIcon = document.createElement('i');
  prevIcon.setAttribute('data-lucide', 'chevron-left');
  prevIcon.className = 'w-5 h-5';
  prevBtn.appendChild(prevIcon);
  prevBtn.addEventListener('click', () => {
    const prev = viewMonth === 0 ? 11 : viewMonth - 1;
    const yr = viewMonth === 0 ? viewYear - 1 : viewYear;
    renderCalendar({ month: prev, year: yr, selectedDate });
  });

  const nextBtn = document.createElement('button');
  nextBtn.className = 'p-2 text-gray-400 hover:text-gray-600';
  const nextIcon = document.createElement('i');
  nextIcon.setAttribute('data-lucide', 'chevron-right');
  nextIcon.className = 'w-5 h-5';
  nextBtn.appendChild(nextIcon);
  nextBtn.addEventListener('click', () => {
    const next = viewMonth === 11 ? 0 : viewMonth + 1;
    const yr = viewMonth === 11 ? viewYear + 1 : viewYear;
    renderCalendar({ month: next, year: yr, selectedDate });
  });

  const monthNames = ['January', 'February', 'March', 'April', 'May', 'June',
    'July', 'August', 'September', 'October', 'November', 'December'];
  const monthLabel = document.createElement('h2');
  monthLabel.className = 'text-lg font-semibold text-gray-900';
  monthLabel.textContent = monthNames[viewMonth] + ' ' + viewYear;

  monthHeader.appendChild(prevBtn);
  monthHeader.appendChild(monthLabel);
  monthHeader.appendChild(nextBtn);
  container.appendChild(monthHeader);

  // Day labels
  const dayLabels = document.createElement('div');
  dayLabels.className = 'grid grid-cols-7 px-4 mb-1';
  ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'].forEach(d => {
    const span = document.createElement('span');
    span.className = 'text-xs text-gray-400 font-medium text-center py-1';
    span.textContent = d;
    dayLabels.appendChild(span);
  });
  container.appendChild(dayLabels);

  // Calendar grid
  const grid = document.createElement('div');
  grid.className = 'grid grid-cols-7 px-4 gap-0.5';

  const firstDay = new Date(viewYear, viewMonth, 1);
  let startDay = firstDay.getDay(); // 0=Sun
  startDay = startDay === 0 ? 6 : startDay - 1; // Convert to Mon=0

  const daysInMonth = new Date(viewYear, viewMonth + 1, 0).getDate();
  const todayStr = today.toISOString().split('T')[0];

  // Appointment dates for dots
  const apptDates = new Set(MOCK_APPOINTMENTS.map(a => a.date));

  // Empty cells before first day
  for (let i = 0; i < startDay; i++) {
    const empty = document.createElement('div');
    empty.className = 'h-10';
    grid.appendChild(empty);
  }

  // Day cells
  for (let day = 1; day <= daysInMonth; day++) {
    const dateStr = viewYear + '-' + String(viewMonth + 1).padStart(2, '0') + '-' + String(day).padStart(2, '0');
    const isToday = dateStr === todayStr;
    const isSelected = dateStr === selectedDate;
    const hasAppt = apptDates.has(dateStr);

    const cell = document.createElement('button');
    cell.className = 'cal-day h-10 flex flex-col items-center justify-center rounded-lg relative text-sm ' +
      (isSelected ? 'selected' : (isToday ? 'today text-indigo-600' : 'text-gray-700'));

    const dayNum = document.createElement('span');
    dayNum.textContent = day;
    cell.appendChild(dayNum);

    if (hasAppt) {
      const dot = document.createElement('span');
      dot.className = 'absolute bottom-1 w-1 h-1 rounded-full ' + (isSelected ? 'bg-white' : 'bg-indigo-500');
      cell.appendChild(dot);
    }

    cell.addEventListener('click', () => {
      renderCalendar({ month: viewMonth, year: viewYear, selectedDate: dateStr });
    });

    grid.appendChild(cell);
  }

  container.appendChild(grid);

  // Appointments for selected date
  const apptSection = document.createElement('div');
  apptSection.className = 'px-4 mt-6';

  const apptTitle = document.createElement('h3');
  apptTitle.className = 'text-sm font-semibold text-gray-900 mb-3';
  const selDate = new Date(selectedDate + 'T00:00:00');
  const dateOpts = { weekday: 'long', month: 'long', day: 'numeric' };
  apptTitle.textContent = selDate.toLocaleDateString('en-US', dateOpts);
  apptSection.appendChild(apptTitle);

  const dayAppts = MOCK_APPOINTMENTS.filter(a => a.date === selectedDate);

  if (dayAppts.length === 0) {
    const emptyMsg = document.createElement('div');
    emptyMsg.className = 'bg-white rounded-2xl shadow-sm p-6 text-center';
    const emptyIcon = document.createElement('div');
    emptyIcon.className = 'text-gray-300 mb-2';
    const calXIcon = document.createElement('i');
    calXIcon.setAttribute('data-lucide', 'calendar-x');
    calXIcon.className = 'w-8 h-8 mx-auto';
    emptyIcon.appendChild(calXIcon);
    emptyMsg.appendChild(emptyIcon);
    const emptyText = document.createElement('p');
    emptyText.className = 'text-sm text-gray-400';
    emptyText.textContent = 'No appointments scheduled';
    emptyMsg.appendChild(emptyText);
    apptSection.appendChild(emptyMsg);
  } else {
    dayAppts.forEach(appt => {
      const card = document.createElement('div');
      card.className = 'bg-white rounded-2xl shadow-sm p-4 mb-2';

      const topRow = document.createElement('div');
      topRow.className = 'flex items-center justify-between mb-2';

      const timeDiv = document.createElement('div');
      timeDiv.className = 'flex items-center gap-2';
      const clockIcon = document.createElement('i');
      clockIcon.setAttribute('data-lucide', 'clock');
      clockIcon.className = 'w-4 h-4 text-gray-400';
      timeDiv.appendChild(clockIcon);
      const timeSpan = document.createElement('span');
      timeSpan.className = 'text-sm font-medium text-gray-900';
      const hour = parseInt(appt.time.split(':')[0]);
      const min = appt.time.split(':')[1];
      const ampm = hour >= 12 ? 'PM' : 'AM';
      const hour12 = hour > 12 ? hour - 12 : (hour === 0 ? 12 : hour);
      timeSpan.textContent = hour12 + ':' + min + ' ' + ampm;
      timeDiv.appendChild(timeSpan);
      const durSpan = document.createElement('span');
      durSpan.className = 'text-xs text-gray-400';
      durSpan.textContent = '(' + appt.duration + ' min)';
      timeDiv.appendChild(durSpan);
      topRow.appendChild(timeDiv);

      const badge = document.createElement('span');
      badge.className = 'text-xs px-2 py-0.5 rounded-full font-medium badge-' + appt.status;
      badge.textContent = appt.status.charAt(0).toUpperCase() + appt.status.slice(1);
      topRow.appendChild(badge);

      card.appendChild(topRow);

      const serviceP = document.createElement('p');
      serviceP.className = 'font-semibold text-sm text-gray-900';
      serviceP.textContent = appt.service;
      card.appendChild(serviceP);

      const providerP = document.createElement('p');
      providerP.className = 'text-sm text-gray-500 mt-0.5';
      providerP.textContent = 'with ' + appt.provider;
      card.appendChild(providerP);

      if (appt.notes) {
        const notesP = document.createElement('p');
        notesP.className = 'text-xs text-gray-400 mt-1';
        notesP.textContent = appt.notes;
        card.appendChild(notesP);
      }

      apptSection.appendChild(card);
    });
  }

  container.appendChild(apptSection);

  main.innerHTML = '';
  main.appendChild(container);
  lucide.createIcons();
}
