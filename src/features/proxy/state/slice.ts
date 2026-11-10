import { shallow } from "zustand/shallow";
import type { CoreNotice, CoreStatus } from "../../../services/api/core";
import type { PrivacyStatus } from "../../../services/api/network";
import type { ProfileItem } from "../../../services/api/profiles";
import type { SlicePort } from "../../../shared/state/slice";

export interface ProxyState {
  errors: CoreNotice[];
  coreStatus: CoreStatus | null;
  coreError: string | null;
  privacyStatus: PrivacyStatus | null;
  currentProfile: ProfileItem | null;
  refreshErrors: () => Promise<void>;
  dismissError: (id: number) => Promise<void>;
  refreshCore: (fresh?: boolean) => Promise<void>;
  refreshProfile: () => Promise<void>;
}

interface ProxyStateApi {
  coreStatus: () => Promise<CoreStatus>;
  currentProfile: () => Promise<ProfileItem | null>;
  coreErrors: () => Promise<CoreNotice[]>;
  dismissCoreError: (id: number) => Promise<void>;
}

export function createProxySlice(
  { set, get }: SlicePort<ProxyState>,
  api: ProxyStateApi,
  publishStatus: (
    core: CoreStatus | null,
    privacy: PrivacyStatus | null,
  ) => void,
): ProxyState {
  let coreRefresh: Promise<void> | null = null;
  let profileRefresh: Promise<void> | null = null;
  let errorRequest = 0;
  return {
    errors: [],
    coreStatus: null,
    coreError: null,
    privacyStatus: null,
    currentProfile: null,
    refreshErrors: async () => {
      const request = ++errorRequest;
      const errors = await api.coreErrors();
      if (request === errorRequest) set({ errors });
    },
    dismissError: async (id) => {
      await api.dismissCoreError(id);
      ++errorRequest;
      set((state) => ({
        errors: state.errors.filter((error) => error.id !== id),
      }));
      await get().refreshErrors();
    },
    refreshCore: (fresh = false) => {
      // 写操作完成后不能复用写操作之前已发出的读取。
      if (fresh && coreRefresh) {
        return coreRefresh
          .catch(() => undefined)
          .then(() => get().refreshCore());
      }
      if (coreRefresh) return coreRefresh;
      const request: Promise<void> = (async () => {
        const observed = await api.coreStatus();
        const previous = get().privacyStatus;
        const privacy = observed.privacy ?? null;
        // 嵌套保护值保持引用稳定，相同状态轮询不会唤醒所有订阅者。
        const privacyStatus = shallow(previous, privacy) ? previous : privacy;
        const coreStatus = { ...observed, privacy: privacyStatus };
        publishStatus(coreStatus, privacyStatus);
        if (get().coreError !== null) set({ coreError: null });
      })()
        .catch((error) => {
          // 读取失败应显示未知，不能沿用上一轮的“已开启”。
          publishStatus(null, null);
          set({ coreError: String(error) });
          throw error;
        })
        .finally(() => {
          if (coreRefresh === request) coreRefresh = null;
        });
      coreRefresh = request;
      return request;
    },
    refreshProfile: () => {
      if (profileRefresh) return profileRefresh;
      profileRefresh = api
        .currentProfile()
        .then((currentProfile) => {
          set((state) =>
            shallow(state.currentProfile, currentProfile)
              ? state
              : { currentProfile },
          );
        })
        .finally(() => {
          profileRefresh = null;
        });
      return profileRefresh;
    },
  };
}
