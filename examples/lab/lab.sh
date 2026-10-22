#!/usr/bin/env bash
# The itda lab: self-hosted software behind a sign-in, on fixed local ports.
#
#   lab.sh up       start every service, wait until each answers, seed one
#                   account (lab / labpassword) and a little content in each
#   lab.sh down     stop the services; the data stays for the next `up`
#   lab.sh reset    stop and delete the data, so the next `up` starts cold
#   lab.sh status   one line per service with its address and health
#
# See README.md for what the lab proves and how the tests use it.
set -euo pipefail

cd "$(dirname "$0")"

USER_NAME=lab
PASSWORD=labpassword
GITEA=http://127.0.0.1:8081
REDMINE=http://127.0.0.1:8082
WORDPRESS=http://127.0.0.1:8083
MEDIAWIKI=http://127.0.0.1:8084
INTRANET=http://127.0.0.1:8085
FRAMED=http://127.0.0.1:8086
SCRIPTED=http://127.0.0.1:8087
URLTOKEN=http://127.0.0.1:8088
LOSSY=http://127.0.0.1:8089

compose() {
  docker compose "$@"
}

say() {
  printf '%s\n' "$*" >&2
}

wait_healthy() {
  local service=$1 tries=${2:-120}
  local i state
  for ((i = 0; i < tries; i++)); do
    state=$(docker inspect --format '{{.State.Health.Status}}' "$(compose ps -q "$service")" 2>/dev/null || true)
    if [ "$state" = healthy ]; then
      return 0
    fi
    sleep 2
  done
  say "$service did not become healthy; docker compose logs $service says why"
  return 1
}

seed_gitea() {
  if ! compose exec -T -u git gitea gitea admin user list 2>/dev/null | awk '{print $2}' | grep -qx "$USER_NAME"; then
    compose exec -T -u git gitea gitea admin user create \
      --username "$USER_NAME" --password "$PASSWORD" --email lab@lab.test \
      --admin --must-change-password=false >/dev/null
  fi
  local api="$GITEA/api/v1"
  if ! curl -fsS -u "$USER_NAME:$PASSWORD" "$api/repos/$USER_NAME/intranet" >/dev/null 2>&1; then
    curl -fsS -u "$USER_NAME:$PASSWORD" -X POST "$api/user/repos" \
      -H 'Content-Type: application/json' \
      -d '{"name":"intranet","private":true,"auto_init":true,"description":"the lab repository"}' >/dev/null
  fi
  local count
  count=$(curl -fsS -u "$USER_NAME:$PASSWORD" "$api/repos/$USER_NAME/intranet/issues?state=all&limit=50" | grep -o '"number"' | wc -l | tr -d ' ' || true)
  if [ "$count" = 0 ]; then
    local title
    for title in "10월 정기 점검 안내" "보안 교육 이수 요청" "사내 메신저 업데이트"; do
      curl -fsS -u "$USER_NAME:$PASSWORD" -X POST "$api/repos/$USER_NAME/intranet/issues" \
        -H 'Content-Type: application/json' \
        -d "{\"title\":\"$title\",\"body\":\"the lab seeded this issue\"}" >/dev/null
    done
  fi
}

seed_redmine() {
  compose exec -T -u redmine -e SECRET_KEY_BASE=itda-lab-redmine-secret-key-base redmine bin/rails runner - <<'RUBY' >/dev/null
    if Tracker.count == 0
      ENV["REDMINE_LANG"] = "en"
      Redmine::DefaultData::Loader.load("en")
    end
    user = User.find_by_login("lab") || User.new(login: "lab", firstname: "Lab", lastname: "User", mail: "lab@lab.test", language: "en")
    user.password = user.password_confirmation = "labpassword"
    user.status = User::STATUS_ACTIVE
    user.must_change_passwd = false
    user.admin = true
    user.save!
    project = Project.find_by_identifier("lab") || Project.create!(name: "Lab", identifier: "lab", is_public: false, enabled_module_names: %w[issue_tracking])
    project.trackers = Tracker.all if project.trackers.empty?
    role = Role.find_by_name("Manager")
    Member.create!(project: project, user: user, roles: [role]) unless project.members.exists?(user_id: user.id)
    if project.issues.count == 0
      ["10월 정기 점검 안내", "보안 교육 이수 요청", "사내 메신저 업데이트"].each do |subject|
        Issue.create!(project: project, tracker: project.trackers.first, author: user, subject: subject,
                      description: "the lab seeded this issue", status: IssueStatus.sorted.first, priority: IssuePriority.default || IssuePriority.first)
      end
    end
RUBY
}

seed_wordpress() {
  local wp="compose run --rm -T wpcli wp"
  if ! $wp core is-installed >/dev/null 2>&1; then
    $wp core install --url="$WORDPRESS" --title=Lab --admin_user="$USER_NAME" \
      --admin_password="$PASSWORD" --admin_email=lab@lab.test --skip-email >/dev/null
  fi
  if [ "$($wp post list --post_status=private --format=count 2>/dev/null | tr -d '\r')" = 0 ]; then
    local title
    for title in "10월 정기 점검 안내" "보안 교육 이수 요청" "사내 메신저 업데이트"; do
      $wp post create --post_title="$title" --post_content="the lab seeded this post" \
        --post_status=private --post_author=1 >/dev/null
    done
  fi
}

seed_mediawiki() {
  compose exec -T -u www-data mediawiki sh -c '
    set -e
    if [ ! -f /var/www/data/LocalSettings.php ]; then
      php maintenance/run.php install --dbtype=sqlite --dbpath=/var/www/data \
        --server=http://127.0.0.1:8084 --scriptpath= --lang=en --pass=labpassword \
        --confpath=/var/www/data LabWiki lab >/dev/null
      cat >> /var/www/data/LocalSettings.php <<EOF

# The lab: a private wiki, as intranet wikis usually are.
\$wgGroupPermissions["*"]["read"] = false;
\$wgGroupPermissions["*"]["edit"] = false;
\$wgGroupPermissions["*"]["createaccount"] = false;
\$wgEnableUploads = false;
EOF
    fi
    for title in "10월 정기 점검 안내" "보안 교육 이수 요청" "사내 메신저 업데이트"; do
      printf "%s\n\nthe lab seeded this page\n" "$title" | php maintenance/run.php edit --user=lab --summary=seed "$title" >/dev/null
    done
  '
}

up() {
  local started
  started=$(date +%s)
  compose up -d gitea redmine db wordpress mediawiki intranet framed scripted urltoken lossy
  wait_healthy gitea
  wait_healthy redmine
  wait_healthy wordpress
  wait_healthy mediawiki
  wait_healthy intranet
  wait_healthy framed
  wait_healthy scripted
  wait_healthy urltoken
  wait_healthy lossy
  say "seeding gitea"
  seed_gitea
  say "seeding redmine"
  seed_redmine
  say "seeding wordpress"
  seed_wordpress
  say "seeding mediawiki"
  seed_mediawiki
  say "lab up in $(( $(date +%s) - started )) s"
  status
}

status() {
  local service url
  for service in gitea redmine wordpress mediawiki intranet framed scripted urltoken lossy; do
    case $service in
      gitea) url=$GITEA ;;
      redmine) url=$REDMINE ;;
      wordpress) url=$WORDPRESS ;;
      mediawiki) url=$MEDIAWIKI ;;
      intranet) url=$INTRANET ;;
      framed) url=$FRAMED ;;
      scripted) url=$SCRIPTED ;;
      urltoken) url=$URLTOKEN ;;
      lossy) url=$LOSSY ;;
    esac
    local id state
    id=$(compose ps -q "$service" 2>/dev/null || true)
    if [ -z "$id" ]; then
      state=down
    else
      state=$(docker inspect --format '{{.State.Health.Status}}' "$id" 2>/dev/null || echo unknown)
    fi
    printf '%-10s %-24s %s\n' "$service" "$url" "$state"
  done
}

case ${1:-} in
  up) up ;;
  down) compose down ;;
  reset) compose down -v ;;
  status) status ;;
  *)
    say "usage: lab.sh up | down | reset | status"
    exit 2
    ;;
esac
