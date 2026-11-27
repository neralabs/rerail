export default async function TeamSettingsPage({ params }: { params: Promise<{ team: string }> }) {
  const { team } = await params;
  return <h1>{team} settings</h1>;
}
