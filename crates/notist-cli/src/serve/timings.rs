//! Bounded, in-memory rebuild reports, never part of the publication artifact.
use super::{State, simple};
use notist::timings::Report;
use std::{collections::VecDeque, sync::Arc};
use tiny_http::Request;

pub(super) type History = VecDeque<(u64, Arc<Report>)>;

pub(super) fn retain(history: &mut History, id: u64, report: Report) {
    history.push_back((id, Arc::new(report)));
    while history.len() > 20 {
        history.pop_front();
    }
}

pub(super) fn respond(request: Request, state: &State) {
    let route = request
        .url()
        .split('?')
        .next()
        .unwrap()
        .trim_end_matches('/');
    let suffix = route
        .strip_prefix("/_notist/timings")
        .unwrap_or("")
        .trim_start_matches('/');
    let published = state.read().unwrap();
    let Some(history) = &published.timings else {
        drop(published);
        simple(request, 404, b"Timings are disabled".to_vec(), "text/plain");
        return;
    };
    let ids: Vec<_> = history.iter().map(|(id, _)| *id).collect();
    if suffix == "history.json" {
        let bytes = serde_json::to_vec(&ids).unwrap();
        drop(published);
        simple(request, 200, bytes, "application/json");
        return;
    }
    let json = suffix.ends_with(".json");
    let id = suffix.trim_end_matches(".json").parse::<u64>().ok();
    let latest = suffix.is_empty();
    let selected = if latest {
        history.back()
    } else {
        id.and_then(|id| history.iter().find(|(build, _)| *build == id))
    };
    let Some((id, report)) = selected.cloned() else {
        drop(published);
        simple(
            request,
            404,
            b"Timing report not retained".to_vec(),
            "text/plain",
        );
        return;
    };
    drop(published);
    if json {
        simple(request, 200, report.json().into_bytes(), "application/json");
        return;
    }
    let controls = format!(
        r#"<script>
const ids={ids}, current={id}, latest={latest};
const history=document.querySelector('#history');
const label=document.createElement('label');label.textContent='Build ';
const select=document.createElement('select');select.setAttribute('aria-label','Build history');
for(const id of ids.slice().reverse()){{const option=document.createElement('option');option.value=id;option.textContent='#'+id;option.selected=id===current;select.append(option)}}
select.onchange=()=>location.href='/_notist/timings/'+select.value;label.append(select);history.append(label);
const follow=document.createElement('a');follow.href='/_notist/timings';follow.textContent=latest?'Following latest completed build':'Follow latest';history.append(follow);
const download=document.createElement('a');download.href='/_notist/timings/'+current+'.json';download.textContent='JSON';history.append(download);
document.title='Notist timings #'+current;
if(latest)setInterval(async()=>{{try{{const response=await fetch('/_notist/timings/history.json');if(response.ok){{const ids=await response.json();if(ids.at(-1)!==current)location.reload()}}}}catch{{}}}},1000);
</script>"#,
        ids = serde_json::to_string(&ids).unwrap()
    );
    let mut html = Vec::new();
    report.write_html(&mut html).unwrap();
    let end = html
        .windows(7)
        .rposition(|part| part == b"</html>")
        .unwrap();
    html.splice(end..end, controls.bytes());
    simple(request, 200, html, "text/html; charset=utf-8");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_evicts_only_the_oldest_completed_reports() {
        let mut history = History::new();
        for id in 1..=25 {
            retain(&mut history, id, Report::default());
        }
        assert_eq!(history.len(), 20);
        assert_eq!(history.front().unwrap().0, 6);
        assert_eq!(history.back().unwrap().0, 25);
    }
}
