// Valeurs par défaut Supabase, commitées : la clé anon est PUBLIQUE par design
// (embarquée dans chaque build distribué), la sécurité vient des policies RLS.
// Un .env local avec VITE_SUPABASE_URL / VITE_SUPABASE_ANON_KEY reste prioritaire
// (utile pour pointer un autre projet en dev).
export const DEFAULT_SUPABASE_URL = "https://ahvlnejmuouibvjdical.supabase.co";
export const DEFAULT_SUPABASE_ANON_KEY = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6ImFodmxuZWptdW91aWJ2amRpY2FsIiwicm9sZSI6ImFub24iLCJpYXQiOjE3ODMxNjQyMDMsImV4cCI6MjA5ODc0MDIwM30.3yAQya7_YflJteExoXttZHjDp2cqaXNfJAz3n8R6j5o";
