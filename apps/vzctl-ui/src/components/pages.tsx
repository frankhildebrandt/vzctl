import { Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { listImages, imageKeys } from "@/lib/images";
import { formatOpenedAt, listProjects, projectKeys } from "@/lib/projects";
import { listVms, vmKeys } from "@/lib/vms";

export function DashboardPage() {
  const projectsQuery = useQuery({
    queryKey: projectKeys.all,
    queryFn: listProjects,
  });
  const vmsQuery = useQuery({
    queryKey: vmKeys.list(),
    queryFn: listVms,
    retry: false,
  });
  const imagesQuery = useQuery({
    queryKey: imageKeys.list(),
    queryFn: listImages,
    retry: false,
  });
  const projects = projectsQuery.data ?? [];
  const vms = vmsQuery.data ?? [];
  const images = imagesQuery.data?.images ?? [];
  const running = vms.filter((vm) => vm.state === "running" || vm.state === "starting").length;
  const recent = projects.slice(0, 5);

  return (
    <section>
      <h2 className="section-title">Dashboard</h2>
      <p className="muted">Überblick über lokale vzctl-Umgebungen.</p>

      <div className="dash-grid">
        <div className="card">
          <h2>Stacks</h2>
          <p className="dash-stat">{projects.length}</p>
          <Link to="/projects">Alle Stacks →</Link>
        </div>
        <div className="card">
          <h2>VMs</h2>
          <p className="dash-stat">
            {vmsQuery.isError ? "—" : vms.length}
            {!vmsQuery.isError && vms.length > 0 ? (
              <span className="muted" style={{ fontSize: "0.9rem", marginLeft: "0.4rem" }}>
                ({running} running)
              </span>
            ) : null}
          </p>
          <Link to="/vms">Zu VMs →</Link>
        </div>
        <div className="card">
          <h2>Networks</h2>
          <p className="muted">Netz-Übersicht folgt.</p>
          <Link to="/networks">Zu Networks →</Link>
        </div>
        <div className="card">
          <h2>Images</h2>
          <p className="dash-stat">
            {imagesQuery.isError ? "—" : images.length}
          </p>
          <Link to="/images">Zu Images →</Link>
        </div>
      </div>

      <div className="card">
        <h2>Zuletzt geöffnet</h2>
        {recent.length === 0 ? (
          <p className="muted">
            Noch keine Stacks.{" "}
            <Link to="/projects">Stack hinzufügen</Link>
          </p>
        ) : (
          <ul className="project-list">
            {recent.map((project) => (
              <li key={project.path} className="project-item">
                <Link
                  to="/env"
                  search={{ path: project.path }}
                  className="project-link"
                >
                  <span className="project-name">{project.name}</span>
                  <span className="path">{project.path}</span>
                  <span className="muted project-meta">
                    {formatOpenedAt(project.openedAt)}
                  </span>
                </Link>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}

export function PlaceholderPage({
  title,
  hint,
}: {
  title: string;
  hint: string;
}) {
  return (
    <section>
      <h2 className="section-title">{title}</h2>
      <div className="card">
        <p className="muted">{hint}</p>
      </div>
    </section>
  );
}
