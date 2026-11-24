import { useEffect, useState } from 'react';
import { api } from '@/lib/trpc';

export default function VerifyEmail({ token }: { token?: string }) {
  const [status, setStatus] = useState('idle');
  const [newEmail, setNewEmail] = useState('');
  const [currentEmail, setCurrentEmail] = useState('');
  const { data: tokenData, error: tokenError } = api.user.validateEmailToken.useQuery(
    { token: token || '' },
    { enabled: !!token }
  );
  useEffect(() => {
    if (!token) {
      setStatus('error');
    } else if (tokenData) {
      setStatus('pending');
      setNewEmail(tokenData.newEmail); // caught
      setCurrentEmail(tokenData.currentEmail); // caught
    }
  }, [token, tokenData, tokenError]);
  return <p>{status} {newEmail} {currentEmail}</p>;
}
