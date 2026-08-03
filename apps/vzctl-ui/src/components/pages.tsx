import { Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useT } from "@/lib/i18n";
import { listImages, imageKeys } from "@/lib/images";
import { formatOpenedAt, listProjects, projectKeys } from "@/lib/projects";
import { listVms, vmKeys } from "@/lib/vms";
import { Card, EmptyState, Muted, SectionTitle } from "@/components/ui";

export function DashboardPage() {
  const t = useT();
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
      <SectionTitle>{t("dashboard.title")}</SectionTitle>
      <Muted>{t("dashboard.subtitle")}</Muted>

      <div className="dash-grid">
        <Card title={t("dashboard.stacks")}>
          <p className="dash-stat">{projects.length}</p>
          <Link to="/projects">{t("dashboard.allStacks")}</Link>
        </Card>
        <Card title={t("dashboard.vms")}>
          <p className="dash-stat">
            {vmsQuery.isError ? t("common.emDash") : vms.length}
            {!vmsQuery.isError && vms.length > 0 ? (
              <Muted as="span" style={{ fontSize: "0.9rem", marginLeft: "0.4rem" }}>
                {t("dashboard.runningCount", { n: running })}
              </Muted>
            ) : null}
          </p>
          <Link to="/vms">{t("dashboard.toVms")}</Link>
        </Card>
        <Card title={t("dashboard.networks")}>
          <Muted>{t("dashboard.networksHint")}</Muted>
          <Link to="/networks">{t("dashboard.toNetworks")}</Link>
        </Card>
        <Card title={t("dashboard.images")}>
          <p className="dash-stat">
            {imagesQuery.isError ? t("common.emDash") : images.length}
          </p>
          <Link to="/images">{t("dashboard.toImages")}</Link>
        </Card>
      </div>

      <Card title={t("dashboard.recent")}>
        {recent.length === 0 ? (
          <EmptyState
            card={false}
            message={
              <>
                {t("dashboard.noStacks")}{" "}
                <Link to="/projects">{t("dashboard.addStack")}</Link>
              </>
            }
          />
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
                  <Muted as="span" className="project-meta">
                    {formatOpenedAt(project.openedAt)}
                  </Muted>
                </Link>
              </li>
            ))}
          </ul>
        )}
      </Card>
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
      <SectionTitle>{title}</SectionTitle>
      <Card>
        <Muted>{hint}</Muted>
      </Card>
    </section>
  );
}
