import { useEffect, useState } from 'react';
import { Gallery, Home } from './screens';

export function App() {
  const [route, setRoute] = useState(window.location.hash);
  useEffect(() => {
    const on = () => setRoute(window.location.hash);
    window.addEventListener('hashchange', on);
    return () => window.removeEventListener('hashchange', on);
  }, []);
  return route === '#/gallery' ? <Gallery /> : <Home />;
}
