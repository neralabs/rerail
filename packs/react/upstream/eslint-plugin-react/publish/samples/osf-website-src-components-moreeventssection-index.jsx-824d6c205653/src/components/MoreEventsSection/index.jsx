import React from 'react'
import './styles.scss';
import LinkComponent from '../LinkComponent';

const COLORS = [
  "#e61e24",
  "#f4a93a",
  "#39ae4a",
  "#28a4db"
];

const MoreEventsSection = ({ title, events }) => {
  return (
    <section className="more-events-section-wrapper" id="community-events">
      <div className="container">
        <h1 className="title" dangerouslySetInnerHTML={{ __html: title }} />
        <div className="more-agenda-list">
          {events.map((event, idx) => (
            <div className="agenda-row" style={{ borderLeftColor: COLORS[idx % 4] }}>
              <div className="agenda-name">
                {event.link ?
                  <>
                    <LinkComponent href={event.link}>{event.name}</LinkComponent>
                  </>
                  :
                  event.name}
              </div>
              <div className="agenda-date">
                <img src="/img/openinfra-days/calendar.svg" alt="date" />
                {event.date}
              </div>
              <div className="agenda-location">
                <img src="/img/openinfra-days/location.svg" alt="location" />
                {event.location}
              </div>
            </div>
          ))}
        </div>

      </div>
    </section>
  )
}

export default MoreEventsSection