# Glossary

- **Course number**: TISS identifier, three digits then three alphanumerics, written `104.299` or `185.A91`. Stored and sent undotted.
- **Course query**: what a user types to name a course, either a course number or part of a course name.
- **Course name**: course type plus title as TISS reports it, such as `VO Mathematik 2`. The type stays in, because it tells VO and UE of one subject apart.
- **Tracked course**: a course saved locally by `tiss track`, whose exams `tiss list` shows.
- **Known course**: a course TISS reported during the current semester, in a search or as a last offering. Known courses are not asked for again until the next semester.
- **Last offering**: the newest semester, at most eight back, in which TISS has a course. TISS search only finds current courses, so a course no longer offered, which can still have exams, is only reachable by its course number.
- **Semester**: `2027S` runs March to September 2027, `2026W` runs October 2026 to February 2027. Courses do not change within one.
- **Fresh exams**: stored exams less than an hour old, shown without asking TISS.
- **Exam date**: when the exam begins (`examinationBegin`). TISS omits it for some exams, shown as `TBA`.
- **Registration deadline**: last moment to register (`applicationEnd`). Shown in red once passed.
- **Exam gap**: calendar days from the exam before to this one, or from today for the first. Shown as `12d` below 14 days, then in whole weeks as `3w`. Exams without a date have none.
- **Upcoming exam**: an exam whose date, or registration deadline if it has no date, is not in the past.
