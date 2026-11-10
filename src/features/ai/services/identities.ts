import { call } from "../../../services/api/invoke";

export interface Retention {
  history: boolean;
  cache: boolean;
  settings: boolean;
  skills: boolean;
  plugins: boolean;
  mcp: boolean;
  projects: boolean;
}
export interface Identity {
  id: string;
  name: string;
  email: string | null;
  createdAt: string;
  bytes: number;
  policy: Retention;
  browserSlot: string;
}
export interface IdentityStatus {
  catalog: {
    records: Identity[];
    active: string | null;
    browserSlot: string;
    recovery: string | null;
  };
  currentEmail: string | null;
  customDirectory: boolean;
}
export const identityEnvironment = {
  identities: () => call<IdentityStatus>("identity_status"),
  identityAction: (
    action: string,
    id: string | null,
    name: string,
    policy: Retention,
  ) =>
    call<{ message: string }>("identity_action", {
      action,
      id,
      name,
      policy,
      confirmed: true,
    }),
};
