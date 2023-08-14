import { createContext, useContext, useEffect } from "react";
import { Wfm, Settings } from '$types/index';
import { settings as sStore, user as uStore } from "@store/index";
import { useStorage } from "../hooks/useStorage.hook";
import { isPermissionGranted, requestPermission, sendNotification } from '@tauri-apps/api/notification';
import { Button } from "@mantine/core";
import { SplashScreen } from "../components/splashScreen";
let permissionGranted = await isPermissionGranted();
if (!permissionGranted) {
  const permission = await requestPermission();
  permissionGranted = permission === 'granted';
}


type TauriContextProps = {
  loading: boolean;
  user: Wfm.UserDto;
  updateUser: (user: Partial<Wfm.UserDto>) => void;
  settings: Settings;
  updateSettings: (user: Partial<Settings>) => void;
  sendNotification: (title: string, body: string) => void;
}
type TauriContextProviderProps = {
  children: React.ReactNode;
}

export const TauriContext = createContext<TauriContextProps>({
  loading: true,
  user: uStore.defaults,
  updateUser: () => { },
  settings: sStore.defaults,
  updateSettings: () => { },
  sendNotification: () => { },
});

export const useTauriContext = () => useContext(TauriContext);

export const TauriContextProvider = ({ children }: TauriContextProviderProps) => {
  const [user, loadingUser, setUser] = useStorage<Wfm.UserDto>(uStore.name, useContext(TauriContext).user);
  const [settings, loadingSetting, setSettings] = useStorage<Settings>(sStore.name, useContext(TauriContext).settings);

  const handleUpdateUser = (userData: Partial<Wfm.UserDto>) => {
    setUser({ ...user, ...userData });
  }

  const handleUpdateSettings = (settingsData: Partial<Settings>) => {
    setSettings({ ...settings, ...settingsData });
  }
  const handleSendNotification = async (title: string, body: string) => {
    if (permissionGranted) {
      sendNotification({ title: title, body: body });
    }
  }

  useEffect(() => {
    if (loadingUser && loadingSetting) {

    }
  }, [loadingUser, loadingSetting]);



  // useEffect(() => {
  //   if (settings.access_token) {
  //     api.auth.isTokenValid().then(async (res) => {
  //       if (!res) {
  //         await uStore.reset();
  //         window.location.reload();
  //       }
  //     })
  //   }
  // }, [settings.access_token]);
  return (
    <TauriContext.Provider value={{ loading: (loadingSetting || loadingUser), user, updateUser: handleUpdateUser, settings, updateSettings: handleUpdateSettings, sendNotification: handleSendNotification }}>
      <SplashScreen opened={loadingSetting || loadingUser} />
      {children}
      <Button onClick={async () => {
        await sStore.reset()
        await uStore.reset()
        // await cache.reset()
        window.location.reload()
      }}>Clear Data</Button>
    </TauriContext.Provider>
  )
}