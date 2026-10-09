import type {
  AdapterCapabilityProfile,
  AdapterDelivery,
  AdapterEvent,
  AdapterSample,
  AdapterSession,
} from "../../src/adapter-contract.js";
import {
  RerailHostRuntime,
  type HostRuntimePort,
  type HostRuntimeSessionState,
} from "../../src/host-runtime.js";
import type {
  HostAdapterContractDriver,
  PersistedContractEntry,
} from "./host-adapter-contract.js";

class ReferenceRuntimePort implements HostRuntimePort {
  readonly samples: AdapterSample[] = [];
  readonly deliveries: AdapterDelivery[] = [];
  readonly persisted: PersistedContractEntry[] = [];
  readonly closedSessions: string[] = [];
  modelLoads = 0;

  async openSession(session: AdapterSession): Promise<HostRuntimeSessionState> {
    this.modelLoads += 1;
    return {
      processedEventIds: this.persisted
        .filter(entry => entry.sessionId === session.sessionId && entry.kind === "event")
        .map(entry => entry.id),
    };
  }

  async closeSession(sessionId: string): Promise<void> {
    this.closedSessions.push(sessionId);
  }

  async persistEvent(event: AdapterEvent): Promise<void> {
    const sessionId = event.kind === "session-start" ? event.session.sessionId : event.sessionId;
    this.persisted.push({ sessionId, kind: "event", id: event.eventId });
  }

  async persistTurn(sessionId: string, eventId: string): Promise<void> {
    this.persisted.push({ sessionId, kind: "turn", id: eventId });
  }

  async persistDelivery(delivery: AdapterDelivery): Promise<void> {
    this.persisted.push({
      sessionId: delivery.sessionId,
      kind: "delivery",
      id: `${delivery.requested}:${this.persisted.length}`,
      data: delivery.content,
    });
  }

  async acceptSample(sample: AdapterSample): Promise<void> {
    this.samples.push(sample);
  }

  async deliver(delivery: AdapterDelivery): Promise<void> {
    this.deliveries.push(delivery);
  }
}

export class ReferenceHostAdapter implements HostAdapterContractDriver {
  readonly profile: AdapterCapabilityProfile;
  readonly #port = new ReferenceRuntimePort();
  readonly #runtime: RerailHostRuntime;

  constructor(profile: AdapterCapabilityProfile) {
    this.profile = profile;
    this.#runtime = new RerailHostRuntime(profile, this.#port);
  }

  start(session: AdapterSession): Promise<void> {
    return this.#runtime.start(session);
  }

  async restart(session: AdapterSession): Promise<void> {
    await this.stop(session.sessionId);
    await this.start(session);
  }

  stop(sessionId: string): Promise<void> {
    return this.#runtime.stop(sessionId);
  }

  emit(event: AdapterEvent): Promise<void> {
    return this.#runtime.ingest(event);
  }

  requestSteer(
    sessionId: string,
    requested: AdapterDelivery["requested"],
    content: string,
  ): Promise<void> {
    return this.#runtime.requestSteer(sessionId, requested, content);
  }

  samples(): readonly AdapterSample[] {
    return this.#port.samples;
  }

  deliveries(): readonly AdapterDelivery[] {
    return this.#port.deliveries;
  }

  persisted(): readonly PersistedContractEntry[] {
    return this.#port.persisted;
  }

  modelLoads(): number {
    return this.#port.modelLoads;
  }
}
