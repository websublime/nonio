// The patient page: who the patient is, their notes, and where staff go next.
import { PatientHeader } from './PatientHeader';
import { PatientNotes } from './PatientNotes';
import { PatientSidebar } from './PatientSidebar';

export function PatientPage({ patientId }: { patientId: string }) {
  return (
    <div className="patient-page">
      <header>
        <h1>Patient</h1>
        <PatientHeader patientId={patientId} />
      </header>
      <aside><PatientSidebar /></aside>
      <main><PatientNotes /></main>
    </div>
  );
}
