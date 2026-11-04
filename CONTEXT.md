# Glossary

- **Course number**: TISS identifier, three digits then three alphanumerics, written `104.299` or `185.A91`. Stored and sent undotted.
- **Course query**: what a user types to name a course, either a course number or part of a course name.
- **Course name**: course type plus title as TISS reports it, such as `VO Mathematik 2`. The type stays in, because it tells VO and UE of one subject apart.
- **Tracked course**: a course saved locally by `tiss add`. Only tracked courses are stored; exams are always fetched live.
- **Exam date**: when the exam begins (`examinationBegin`). TISS omits it for some exams, shown as `TBA`.
- **Registration deadline**: last moment to register (`applicationEnd`). Shown as `(closed)` once passed.
- **Upcoming exam**: an exam whose date, or registration deadline if it has no date, is not in the past.
