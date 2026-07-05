-- 관리자 권한 컬럼. 멱등(IF NOT EXISTS).
ALTER TABLE users ADD COLUMN IF NOT EXISTS is_admin boolean NOT NULL DEFAULT false;
-- 기존 admin 계정을 관리자로 승격(초기 관리자 보장).
UPDATE users SET is_admin = true WHERE name = 'admin';
